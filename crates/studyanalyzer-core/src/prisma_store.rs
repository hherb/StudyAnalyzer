//! PRISMA 2020 runs on one paper (design 2026-11-18, §7), stored part by
//! part. A run starts with its part plan stored as `pending` parts; each part
//! is stored the moment it finishes, so a run that is stopped or fails keeps
//! what it finished and Continue runs only what is left. A run is on one
//! canonical text, so every stored span points into it.
//!
//! One shown row per checklist item: the row a non-`recheck` part stored. A
//! re-check (§6.5) stores its own row too, and saving it swaps the answers:
//! the shown row takes the re-check's, the re-check's row keeps the one it
//! replaced ([`save_part`]), so every answer stays stored, each once.

use std::collections::HashSet;

use bmlib::db::{execute, fetch_all, fetch_one, transaction_with, Db, Row, Value};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::quality::prisma2020_checklist::{item as checklist_item, CHECKLIST, KEY_ITEMS};
use bmlib::quality::prisma2020_models::{read_suitability, Judgement};
use bmlib::quality::{ItemAssessment, PartAssessment, PartPlan, Suitability};
use serde::Serialize;

use crate::pico_store::{
    citation_from, contradicted_from_json, contradicted_to_json, from_json, optional, text_of,
    to_json,
};
use crate::store::{canonical_by_id, last_id, PaperId, StoreError};

// The stored vocabulary below mirrors the `CHECK` constraints of migration 13
// (`schema.rs`); it is the schema's, not a setting, so it lives here.

/// A part from the run's starting plan.
pub const KIND_PLAN: &str = "plan";
/// One half of a part whose reply was cut off at its limit (a
/// [`KIND_MISSING`] part's halves stay [`KIND_MISSING`]).
pub const KIND_HALF: &str = "half";
/// The items a reply left out, asked again.
pub const KIND_MISSING: &str = "missing";
/// One key item judged from a condensation, checked again (§6.5).
pub const KIND_RECHECK: &str = "recheck";
/// A part not yet assessed: Continue runs it.
pub const PART_PENDING: &str = "pending";
/// A part assessed and stored, with its items.
pub const PART_DONE: &str = "done";
/// A part replaced by two halves; it holds no items.
pub const PART_SPLIT: &str = "split";
/// A part whose call failed after its retries, or a follow-up stored failed
/// because a reply, asked again, still left its items out
/// ([`ERROR_MISSING_ITEMS`]); it holds no items.
pub const PART_FAILED: &str = "failed";
/// The error kind of a part whose reply, asked again, still left items out.
pub const ERROR_MISSING_ITEMS: &str = "missing_items";
/// A run with parts still pending or failed.
pub const RUN_INCOMPLETE: &str = "incomplete";
/// A run with every part done or split, or one that stopped at its check.
pub const RUN_COMPLETE: &str = "complete";

/// A suitability check and what it read.
#[derive(Debug, Clone, PartialEq)]
pub struct PrismaCheck {
    /// The model's verdict.
    pub suitability: Suitability,
    /// True when the check read the paper's abstract, false for the whole text.
    pub from_abstract: bool,
}

/// A part still to assess, as [`pending_parts`] gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPart {
    /// The part's row id.
    pub id: i64,
    /// Its kind: [`KIND_PLAN`], [`KIND_HALF`], [`KIND_MISSING`] or [`KIND_RECHECK`].
    pub kind: String,
    /// The checklist ids it asks, in checklist order.
    pub item_ids: Vec<String>,
}

/// One stored part of a run, whatever its status.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPrismaPart {
    /// The part's row id.
    pub id: i64,
    /// Its kind ([`KIND_PLAN`] and the rest).
    pub kind: String,
    /// The checklist ids it asks, in checklist order.
    pub item_ids: Vec<String>,
    /// Its status: [`PART_PENDING`], [`PART_DONE`], [`PART_SPLIT`] or [`PART_FAILED`].
    pub status: String,
    /// The paper's length before condensing, when its paper was condensed.
    pub condensed_from_chars: Option<usize>,
    /// How condensing went, when it was condensed.
    pub condensation_status: Option<String>,
    /// Notes for the reader from its assessment.
    pub notes: Vec<String>,
    /// The checklist ids its reply answered although the part did not ask
    /// them; those answers were set aside (bmlib #808).
    pub unasked: Vec<String>,
    /// For a failed part, or one made pending again by [`reset_failed`] and
    /// not yet saved, the error's kind.
    pub error_kind: Option<String>,
    /// The error's message, with `error_kind`.
    pub error_message: Option<String>,
}

/// One shown checklist item of a run.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPrismaItem {
    /// The (non-`recheck`) part that stored it.
    pub part_id: i64,
    /// The item as shown; every citation's `text` re-read from the paper.
    pub assessment: ItemAssessment,
    /// Whether its (non-`recheck`) part's paper was condensed; a re-check
    /// swaps the answer but not this.
    pub condensed: bool,
    /// For a re-checked item, the judgement shown before its first re-check.
    pub first_judgement: Option<Judgement>,
    /// For a re-checked item, the latest re-check part (its condensation).
    pub recheck_part_id: Option<i64>,
}

/// A stored run, as [`run`] reads it back.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPrismaRun {
    /// The run's row id.
    pub id: i64,
    /// The canonical text it ran on.
    pub canonical_id: i64,
    /// The model that ran it.
    pub model: String,
    /// When it was started (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// Its suitability check, or `None` when there was none.
    pub check: Option<PrismaCheck>,
    /// Whether the user ran it without a passed check ("Run anyway").
    pub forced: bool,
    /// The review variant the check named, as the model wrote it.
    pub variant: Option<String>,
    /// [`RUN_INCOMPLETE`] or [`RUN_COMPLETE`].
    pub status: String,
    /// Every part, in id order (re-checks and failed parts included).
    pub parts: Vec<StoredPrismaPart>,
    /// One shown row per assessed checklist item, in checklist order. Items
    /// not assessed yet have none.
    pub items: Vec<StoredPrismaItem>,
    /// The canonical text it ran on, as read to check its spans.
    pub text: CanonicalText,
}

/// One line of a paper's list of PRISMA runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrismaRunSummary {
    /// The run's row id.
    pub id: i64,
    /// The canonical text it ran on.
    pub canonical_id: i64,
    /// The model that ran it.
    pub model: String,
    /// When it was started (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// [`RUN_INCOMPLETE`] or [`RUN_COMPLETE`].
    pub status: String,
}

/// Refuses an empty list of ids or an id the checklist does not hold.
fn check_ids(ids: &[&str]) -> Result<(), StoreError> {
    if ids.is_empty() {
        return Err(StoreError::Corrupt(
            "a part must ask at least one item".into(),
        ));
    }
    match ids.iter().find(|id| checklist_item(id).is_none()) {
        Some(id) => Err(StoreError::Corrupt(format!(
            "{id:?} is not a PRISMA 2020 checklist item"
        ))),
        None => Ok(()),
    }
}

/// `ids` as a JSON array.
fn ids_json(ids: &[&str]) -> Result<String, StoreError> {
    serde_json::to_string(ids)
        .map_err(|e| StoreError::Corrupt(format!("item ids cannot be stored: {e}")))
}

/// The canonical text `canonical_id` of `paper`; one that is not the paper's
/// is [`StoreError::Corrupt`].
fn text_of_paper(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
) -> Result<CanonicalText, StoreError> {
    Ok(canonical_by_id(db, paper, canonical_id)?
        .ok_or_else(|| {
            StoreError::Corrupt(format!(
                "canonical text {canonical_id} is not paper {}'s",
                paper.as_str()
            ))
        })?
        .text)
}

/// Inserts a run row with `status` and returns its id.
fn insert_run(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
    model: &str,
    check: Option<&PrismaCheck>,
    forced: bool,
    status: &str,
) -> Result<i64, StoreError> {
    execute(
        db,
        "INSERT INTO prisma_runs (paper_id, canonical_id, model, suitability_json,
                                  suitability_from_abstract, forced, variant, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::from(paper.as_str()),
            Value::from(canonical_id),
            Value::from(model),
            Value::from(check.map(|c| c.suitability.to_json().to_string())),
            Value::from(check.map(|c| c.from_abstract)),
            Value::from(forced),
            Value::from(check.and_then(|c| c.suitability.variant.clone())),
            Value::from(status),
        ],
    )?;
    last_id(db)
}

/// Inserts a `pending` part of `kind` asking `ids` and returns its id.
fn insert_part(db: &mut dyn Db, run_id: i64, kind: &str, ids: &[&str]) -> Result<i64, StoreError> {
    execute(
        db,
        "INSERT INTO prisma_parts (run_id, kind, item_ids_json, status) VALUES (?, ?, ?, ?)",
        &[
            Value::from(run_id),
            Value::from(kind),
            Value::from(ids_json(ids)?),
            Value::from(PART_PENDING),
        ],
    )?;
    last_id(db)
}

/// Starts a run on canonical text `canonical_id` of `paper`: the run is
/// stored `incomplete`, with one `pending` [`KIND_PLAN`] part per `plan`
/// entry in order (§7: the split is stored when the run starts, so Continue
/// uses it). Returns the run's id. A run that is not forced needs a passed
/// `check`. An empty plan, a part with no items or an unknown item, an item
/// in two parts (such a run could never complete), a text that is not
/// `paper`'s, or a check that does not allow the run is
/// [`StoreError::Corrupt`], and nothing is written.
pub fn create_run(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
    model: &str,
    check: Option<&PrismaCheck>,
    forced: bool,
    plan: &[PartPlan],
) -> Result<i64, StoreError> {
    if !forced && !check.is_some_and(|c| c.suitability.is_suitable) {
        return Err(StoreError::Corrupt(
            "a run that is not forced needs a passed check".into(),
        ));
    }
    if plan.is_empty() {
        return Err(StoreError::Corrupt("a run needs at least one part".into()));
    }
    let mut seen = HashSet::new();
    for part in plan {
        check_ids(&part.item_ids)?;
        if let Some(id) = part.item_ids.iter().find(|id| !seen.insert(**id)) {
            return Err(StoreError::Corrupt(format!(
                "item {id} is in two parts of the plan"
            )));
        }
    }
    text_of_paper(db, paper, canonical_id)?;
    transaction_with(db, |tx| {
        let run_id = insert_run(
            tx,
            paper,
            canonical_id,
            model,
            check,
            forced,
            RUN_INCOMPLETE,
        )?;
        for part in plan {
            insert_part(tx, run_id, KIND_PLAN, &part.item_ids)?;
        }
        Ok(run_id)
    })
}

/// Stores a run that stopped at a failed `check`: no parts, `complete`.
/// Returns its id. A passed check, or a text that is not `paper`'s, is
/// [`StoreError::Corrupt`], never written.
pub fn save_not_suitable(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
    model: &str,
    check: &PrismaCheck,
) -> Result<i64, StoreError> {
    if check.suitability.is_suitable {
        return Err(StoreError::Corrupt(
            "a run that stopped at a passed check cannot be stored".into(),
        ));
    }
    text_of_paper(db, paper, canonical_id)?;
    insert_run(
        db,
        paper,
        canonical_id,
        model,
        Some(check),
        false,
        RUN_COMPLETE,
    )
}

/// The run's `pending` parts in id order: what is left to assess.
pub fn pending_parts(db: &mut dyn Db, run_id: i64) -> Result<Vec<PendingPart>, StoreError> {
    let rows = fetch_all(
        db,
        "SELECT id, kind, item_ids_json FROM prisma_parts
         WHERE run_id = ? AND status = ? ORDER BY id",
        &[Value::from(run_id), Value::from(PART_PENDING)],
    )?;
    rows.iter()
        .map(|r| {
            let id = r.get_i64("id")?;
            Ok(PendingPart {
                id,
                kind: r.get_str("kind")?.to_string(),
                item_ids: from_json(r.get_str("item_ids_json")?)
                    .map_err(|e| StoreError::Corrupt(format!("PRISMA part {id}: {e}")))?,
            })
        })
        .collect()
}

/// A part row as the writers need it.
struct PartRow {
    /// The run it belongs to.
    run_id: i64,
    /// Its kind.
    kind: String,
    /// The checklist ids it asks.
    item_ids: Vec<String>,
}

/// Part `part_id`, which must be `pending`: a part that is done (it holds
/// items), split or failed is never changed again, except by [`reset_failed`].
/// No such part is [`StoreError::NotFound`]; one not pending is
/// [`StoreError::Corrupt`].
fn pending_part(db: &mut dyn Db, part_id: i64) -> Result<PartRow, StoreError> {
    let r = fetch_one(
        db,
        "SELECT run_id, kind, item_ids_json, status FROM prisma_parts WHERE id = ?",
        &[Value::from(part_id)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("PRISMA part {part_id}")))?;
    let status = r.get_str("status")?;
    if status != PART_PENDING {
        return Err(StoreError::Corrupt(format!(
            "PRISMA part {part_id} is {status}, not pending"
        )));
    }
    Ok(PartRow {
        run_id: r.get_i64("run_id")?,
        kind: r.get_str("kind")?.to_string(),
        item_ids: from_json(r.get_str("item_ids_json")?)
            .map_err(|e| StoreError::Corrupt(format!("PRISMA part {part_id}: {e}")))?,
    })
}

/// Stores pending part `part_id`'s `assessment` in one transaction: its items
/// and their citations, its condensation, notes and the ids it answered
/// unasked, and the part as `done`, any error an earlier attempt left cleared. Whether its paper was condensed
/// is `assessment.condensed_from_chars`, so the item rows and the part never
/// disagree.
///
/// The ids the reply left out (`assessment.missing`) get their follow-up
/// part in the same transaction, so an item never drops out of a run
/// unnoticed (§6.4), even if the app stops right after:
/// - after a [`KIND_PLAN`] or [`KIND_HALF`] part, a pending [`KIND_MISSING`]
///   part asks them again;
/// - after a [`KIND_MISSING`] part (its one re-ask is spent), a
///   [`KIND_MISSING`] part is stored already failed, with error kind
///   [`ERROR_MISSING_ITEMS`] and a message naming them;
/// - after a [`KIND_RECHECK`] part, a [`KIND_RECHECK`] part is stored failed
///   the same way.
///
/// A failed follow-up keeps the run incomplete; Continue's [`reset_failed`]
/// makes it pending again. Each follow-up is logged with its items, so the
/// model's omissions reach the log even when the re-ask cures them.
///
/// Re-checks (§6.5) are kept in the same transaction too:
/// - after a condensed part that is not a re-check, each of its
///   items in [`KEY_ITEMS`] shown `not_reported` (an unbacked claim
///   included) gets a pending [`KIND_RECHECK`] part of its own, in the part's
///   order;
/// - a [`KIND_RECHECK`] part's answer is swapped with the item's shown
///   row's: the model's and the shown judgement, `unbacked`, the
///   explanation, the unlocated quotes and the dropped count, and the
///   citations. The shown row then holds the re-check's answer and the
///   re-check's own row the answer it replaced (the first one, for a first
///   re-check), so both stay stored, each once. The shown judgement before
///   the first re-check is kept in `first_judgement`, never overwritten by a
///   later one; `recheck_part_id` names the latest. Whether the item was
///   judged from a condensation stays each row's own. A re-check never adds
///   another.
///
/// Refused as [`StoreError::Corrupt`], with nothing written: a part not
/// pending, a reply whose items and missing ids are not exactly the ids the
/// part asked (one left in neither, one not asked, or one twice), an item
/// another non-re-check part of the run already shows, or a citation outside
/// the run's text. No such part, or a re-check of an item the run does not
/// show, is [`StoreError::NotFound`].
pub fn save_part(
    db: &mut dyn Db,
    part_id: i64,
    assessment: &PartAssessment,
) -> Result<(), StoreError> {
    let notes = to_json(&assessment.notes)?;
    let unasked = to_json(&assessment.unasked)?;
    let condensed = assessment.condensed_from_chars.is_some();
    // The checks read inside the transaction, so nothing changes between
    // them and the writes.
    transaction_with(db, |tx| {
        let part = pending_part(tx, part_id)?;
        let (paper, canonical_id) = run_text(tx, part.run_id)?
            .ok_or_else(|| StoreError::NotFound(format!("PRISMA run {}", part.run_id)))?;
        let text = text_of_paper(tx, &paper, canonical_id)?;
        let mut answered: Vec<&str> = assessment
            .items
            .iter()
            .map(|i| i.id)
            .chain(assessment.missing.iter().copied())
            .collect();
        let mut asked: Vec<&str> = part.item_ids.iter().map(String::as_str).collect();
        answered.sort_unstable();
        asked.sort_unstable();
        if answered != asked {
            return Err(StoreError::Corrupt(format!(
                "PRISMA part {part_id} asked {:?}, but its reply holds {:?} and leaves out {:?}",
                part.item_ids,
                assessment.items.iter().map(|i| i.id).collect::<Vec<_>>(),
                assessment.missing
            )));
        }
        for item in &assessment.items {
            if part.kind != KIND_RECHECK && shown_row(tx, part.run_id, item.id)?.is_some() {
                return Err(StoreError::Corrupt(format!(
                    "item {} is already assessed in PRISMA run {}",
                    item.id, part.run_id
                )));
            }
            for c in &item.citations {
                let span = TextSpan {
                    start: c.start,
                    end: c.end,
                };
                if text.span_text(span).is_none() {
                    return Err(StoreError::Corrupt(format!(
                        "citation {span} is outside canonical text {canonical_id}"
                    )));
                }
            }
        }
        let mut item_rows = Vec::with_capacity(assessment.items.len());
        for item in &assessment.items {
            execute(
                tx,
                "INSERT INTO prisma_items (part_id, item_id, model_judgement, judgement, unbacked,
                                           explanation, condensed, quotes_not_found_json,
                                           quotes_contradicted_json, quotes_dropped)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                &[
                    Value::from(part_id),
                    Value::from(item.id),
                    Value::from(item.model_judgement.as_str()),
                    Value::from(item.judgement.as_str()),
                    Value::from(item.unbacked),
                    Value::from(item.explanation.as_str()),
                    Value::from(condensed),
                    Value::from(to_json(&item.quotes_not_found)?),
                    Value::from(contradicted_to_json(&item.quotes_contradicted)),
                    Value::from(item.quotes_dropped),
                ],
            )?;
            let item_row = last_id(tx)?;
            item_rows.push(item_row);
            for (position, c) in item.citations.iter().enumerate() {
                execute(
                    tx,
                    "INSERT INTO prisma_citations (prisma_item_id, position, span_start, span_end,
                                                   similarity, exact)
                     VALUES (?, ?, ?, ?, ?, ?)",
                    &[
                        Value::from(item_row),
                        Value::from(position),
                        Value::from(c.start),
                        Value::from(c.end),
                        Value::from(c.similarity),
                        Value::from(c.exact),
                    ],
                )?;
            }
        }
        execute(
            tx,
            "UPDATE prisma_parts SET status = ?, condensed_from_chars = ?, condensation_status = ?,
                                     notes_json = ?, unasked_json = ?, error_kind = NULL,
                                     error_message = NULL
             WHERE id = ?",
            &[
                Value::from(PART_DONE),
                Value::from(assessment.condensed_from_chars),
                Value::from(assessment.condensation_status.clone()),
                Value::from(notes.as_str()),
                Value::from(unasked.as_str()),
                Value::from(part_id),
            ],
        )?;
        if !assessment.missing.is_empty() {
            // In the order the part asked them.
            let missing: Vec<&str> = part
                .item_ids
                .iter()
                .map(String::as_str)
                .filter(|id| assessment.missing.contains(id))
                .collect();
            let (kind, reask) = match part.kind.as_str() {
                KIND_RECHECK => (KIND_RECHECK, false),
                KIND_MISSING => (KIND_MISSING, false),
                _ => (KIND_MISSING, true),
            };
            let follow_up = insert_part(tx, part.run_id, kind, &missing)?;
            if reask {
                log::warn!(
                    "PRISMA run {}: part {part_id}'s reply left out item(s) {}; part {follow_up} asks them again",
                    part.run_id,
                    missing.join(", ")
                );
            } else {
                let message = still_missing(&missing);
                log::warn!(
                    "PRISMA run {}: part {follow_up} failed: {message}",
                    part.run_id
                );
                set_failed(tx, follow_up, ERROR_MISSING_ITEMS, &message)?;
            }
        }
        if part.kind == KIND_RECHECK {
            for (item, &row) in assessment.items.iter().zip(&item_rows) {
                apply_recheck(tx, part.run_id, item.id, part_id, row)?;
            }
        } else if condensed {
            for item in &assessment.items {
                if KEY_ITEMS.contains(&item.id) && item.judgement == Judgement::NotReported {
                    insert_part(tx, part.run_id, KIND_RECHECK, &[item.id])?;
                }
            }
        }
        Ok(())
    })
}

/// The message of a part failed because the model's reply, asked again,
/// still left out `ids`.
fn still_missing(ids: &[&str]) -> String {
    format!(
        "The model's reply still left out item(s) {}.",
        ids.join(", ")
    )
}

/// The row id of the shown row for `item_id` in run `run_id`: the one a
/// non-`recheck` part stored, or `None`.
fn shown_row(db: &mut dyn Db, run_id: i64, item_id: &str) -> Result<Option<i64>, StoreError> {
    fetch_one(
        db,
        "SELECT i.id FROM prisma_items i JOIN prisma_parts p ON p.id = i.part_id
         WHERE p.run_id = ? AND p.kind != ? AND i.item_id = ?",
        &[
            Value::from(run_id),
            Value::from(KIND_RECHECK),
            Value::from(item_id),
        ],
    )?
    .map(|r| r.get_i64("id"))
    .transpose()
    .map_err(StoreError::from)
}

/// Replaces pending part `part_id`, whose reply was cut off, with two
/// pending parts asking `halves`, and marks it `split`, in one transaction.
/// The halves are [`KIND_HALF`], except a [`KIND_MISSING`] part's, which stay
/// [`KIND_MISSING`]: their one re-ask is spent, so an item a half still
/// leaves out fails at once (§6.4). The halves, in order, must ask exactly
/// the part's items in its order, each half at least one; otherwise, or for
/// a part not pending, [`StoreError::Corrupt`].
pub fn split_part(db: &mut dyn Db, part_id: i64, halves: [&[&str]; 2]) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let part = pending_part(tx, part_id)?;
        let joined: Vec<&str> = halves.iter().flat_map(|h| h.iter().copied()).collect();
        if halves.iter().any(|h| h.is_empty()) || joined != part.item_ids {
            return Err(StoreError::Corrupt(format!(
                "halves {halves:?} do not split PRISMA part {part_id}'s items {:?}",
                part.item_ids
            )));
        }
        execute(
            tx,
            "UPDATE prisma_parts SET status = ?, error_kind = NULL, error_message = NULL
             WHERE id = ?",
            &[Value::from(PART_SPLIT), Value::from(part_id)],
        )?;
        let kind = if part.kind == KIND_MISSING {
            KIND_MISSING
        } else {
            KIND_HALF
        };
        for half in halves {
            insert_part(tx, part.run_id, kind, half)?;
        }
        Ok(())
    })
}

/// Marks pending part `part_id` `failed`, keeping the error's `kind` and
/// `message` to show. Only a pending part can fail, so a failed part never
/// holds items: a part that is done (it holds items), split or failed is
/// [`StoreError::Corrupt`]; no such part is [`StoreError::NotFound`].
pub fn fail_part(
    db: &mut dyn Db,
    part_id: i64,
    kind: &str,
    message: &str,
) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        pending_part(tx, part_id)?;
        set_failed(tx, part_id, kind, message)
    })
}

/// Marks part `part_id` `failed` with the error's `kind` and `message`,
/// whatever its status: the callers check it is pending.
fn set_failed(db: &mut dyn Db, part_id: i64, kind: &str, message: &str) -> Result<(), StoreError> {
    execute(
        db,
        "UPDATE prisma_parts SET status = ?, error_kind = ?, error_message = ? WHERE id = ?",
        &[
            Value::from(PART_FAILED),
            Value::from(kind),
            Value::from(message),
            Value::from(part_id),
        ],
    )?;
    Ok(())
}

/// Makes run `run_id`'s failed parts pending again and returns how many:
/// Continue calls it before running what is pending. Each keeps its error
/// until it is saved or split, so a Continue that stops or fails before
/// reaching it still shows why it failed (the view lists it as failed).
pub fn reset_failed(db: &mut dyn Db, run_id: i64) -> Result<usize, StoreError> {
    let n = execute(
        db,
        "UPDATE prisma_parts SET status = ? WHERE run_id = ? AND status = ?",
        &[
            Value::from(PART_PENDING),
            Value::from(run_id),
            Value::from(PART_FAILED),
        ],
    )?;
    usize::try_from(n).map_err(|_| StoreError::Corrupt(format!("{n} parts reset")))
}

/// The columns of an item row that make up its answer: what a re-check
/// swaps between the shown row and its own ([`apply_recheck`]).
const ANSWER_COLUMNS: &str = "model_judgement, judgement, unbacked, explanation, \
    quotes_not_found_json, quotes_contradicted_json, quotes_dropped";

/// Swaps the answers of re-check row `recheck_row` (part `recheck_part_id`'s
/// answer for `item_id`) and the item's shown row in run `run_id`, as
/// [`save_part`] describes: the shown row takes the re-check's answer, and
/// the re-check's row takes the answer it replaced, so both stay stored,
/// each once. The answer is [`ANSWER_COLUMNS`] and the citations; each row
/// keeps its own `condensed`. It runs inside the caller's transaction. No
/// shown row is [`StoreError::NotFound`].
fn apply_recheck(
    db: &mut dyn Db,
    run_id: i64,
    item_id: &str,
    recheck_part_id: i64,
    recheck_row: i64,
) -> Result<(), StoreError> {
    let shown = shown_row(db, run_id, item_id)?.ok_or_else(|| {
        StoreError::NotFound(format!("item {item_id} assessed in PRISMA run {run_id}"))
    })?;
    // Read first: SQLite gives no way to swap two rows in one statement.
    let recheck = fetch_one(
        db,
        &format!("SELECT {ANSWER_COLUMNS} FROM prisma_items WHERE id = ?"),
        &[Value::from(recheck_row)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("PRISMA item row {recheck_row}")))?;
    let mut values = ANSWER_COLUMNS
        .split(',')
        .map(|column| Ok(recheck.get(column.trim())?.clone()))
        .collect::<Result<Vec<Value>, StoreError>>()?;
    execute(
        db,
        &format!(
            "UPDATE prisma_items SET ({ANSWER_COLUMNS}) =
               (SELECT {ANSWER_COLUMNS} FROM prisma_items WHERE id = ?)
             WHERE id = ?"
        ),
        &[Value::from(shown), Value::from(recheck_row)],
    )?;
    // Every right-hand side reads the row before the update, so
    // `first_judgement` takes the judgement being replaced.
    values.extend([Value::from(recheck_part_id), Value::from(shown)]);
    let placeholders = vec!["?"; values.len() - 2].join(", ");
    execute(
        db,
        &format!(
            "UPDATE prisma_items SET
               first_judgement = coalesce(first_judgement, judgement),
               ({ANSWER_COLUMNS}) = ({placeholders}),
               recheck_part_id = ?
             WHERE id = ?"
        ),
        &values,
    )?;
    execute(
        db,
        "UPDATE prisma_citations SET prisma_item_id = CASE prisma_item_id WHEN ? THEN ? ELSE ? END
         WHERE prisma_item_id IN (?, ?)",
        &[
            Value::from(shown),
            Value::from(recheck_row),
            Value::from(shown),
            Value::from(shown),
            Value::from(recheck_row),
        ],
    )?;
    Ok(())
}

/// Marks run `run_id` `complete` when no part is pending or failed, and
/// returns whether it did; with parts left the run stays `incomplete` and
/// the result is `false`. No such run is [`StoreError::NotFound`].
pub fn finish_run(db: &mut dyn Db, run_id: i64) -> Result<bool, StoreError> {
    if run_text(db, run_id)?.is_none() {
        return Err(StoreError::NotFound(format!("PRISMA run {run_id}")));
    }
    let left = fetch_one(
        db,
        "SELECT count(*) AS n FROM prisma_parts WHERE run_id = ? AND status IN (?, ?)",
        &[
            Value::from(run_id),
            Value::from(PART_PENDING),
            Value::from(PART_FAILED),
        ],
    )?
    .map(|r| r.get_i64("n"))
    .transpose()?
    .unwrap_or(0);
    if left > 0 {
        return Ok(false);
    }
    execute(
        db,
        "UPDATE prisma_runs SET status = ? WHERE id = ?",
        &[Value::from(RUN_COMPLETE), Value::from(run_id)],
    )?;
    Ok(true)
}

/// The stored run `run_id`, or `None` if there is none: every part, and one
/// shown row per assessed item (from non-`recheck` parts) in checklist
/// order, every citation checked against the run's text and its `text`
/// re-read from it. A row this code did not write is
/// [`StoreError::Corrupt`]: a missing text, a span outside it, an unknown
/// item or judgement, two shown rows for one item, a check half stored, or
/// a column of the wrong type.
pub fn run(db: &mut dyn Db, run_id: i64) -> Result<Option<StoredPrismaRun>, StoreError> {
    let Some(r) = fetch_one(
        db,
        "SELECT paper_id, canonical_id, model, suitability_json, suitability_from_abstract,
                forced, variant, status, created_at
         FROM prisma_runs WHERE id = ?",
        &[Value::from(run_id)],
    )?
    else {
        return Ok(None);
    };
    let bad = |why: String| StoreError::Corrupt(format!("PRISMA run {run_id}: {why}"));
    let paper = PaperId(r.get_str("paper_id")?.to_string());
    let canonical_id = r.get_i64("canonical_id")?;
    let text = canonical_by_id(db, &paper, canonical_id)?
        .ok_or_else(|| bad("its canonical text is not stored".into()))?
        .text;
    let check = match (
        r.get("suitability_json")?.as_str(),
        r.get("suitability_from_abstract")?.as_i64(),
    ) {
        (Some(json), Some(from_abstract)) => Some(PrismaCheck {
            suitability: suitability_from_json(json).map_err(bad)?,
            from_abstract: from_abstract != 0,
        }),
        (None, None) => None,
        _ => return Err(bad("its check is half stored".into())),
    };
    let part_rows = fetch_all(
        db,
        "SELECT id, kind, item_ids_json, status, condensed_from_chars, condensation_status,
                notes_json, unasked_json, error_kind, error_message
         FROM prisma_parts WHERE run_id = ? ORDER BY id",
        &[Value::from(run_id)],
    )?;
    let parts = part_rows
        .iter()
        .map(|p| part_from(p).map_err(&bad))
        .collect::<Result<Vec<_>, _>>()?;
    let item_rows = fetch_all(
        db,
        "SELECT i.id, i.part_id, i.item_id, i.model_judgement, i.judgement, i.unbacked,
                i.explanation, i.condensed, i.quotes_not_found_json, i.quotes_contradicted_json,
                i.quotes_dropped, i.first_judgement, i.recheck_part_id
         FROM prisma_items i JOIN prisma_parts p ON p.id = i.part_id
         WHERE p.run_id = ? AND p.kind != ?",
        &[Value::from(run_id), Value::from(KIND_RECHECK)],
    )?;
    let mut items = item_rows
        .iter()
        .map(|i| item_from(db, &text, i).map_err(&bad))
        .collect::<Result<Vec<_>, _>>()?;
    let position = |id: &str| CHECKLIST.iter().position(|row| row.id == id);
    items.sort_by_key(|i| position(i.assessment.id));
    if let Some(w) = items
        .windows(2)
        .find(|w| w[0].assessment.id == w[1].assessment.id)
    {
        return Err(bad(format!(
            "item {} has two shown rows",
            w[0].assessment.id
        )));
    }
    Ok(Some(StoredPrismaRun {
        id: run_id,
        canonical_id,
        model: r.get_str("model")?.to_string(),
        created_at: r.get_str("created_at")?.to_string(),
        check,
        forced: r.get_i64("forced")? != 0,
        variant: optional(&r, "variant", text_of).map_err(bad)?,
        status: r.get_str("status")?.to_string(),
        parts,
        items,
        text,
    }))
}

/// A check stored as [`Suitability::to_json`], read back: bmlib's
/// [`read_suitability`] for the model's fields, then the condensation and
/// notes it does not read. A field of the wrong type is an error.
fn suitability_from_json(json: &str) -> Result<Suitability, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("its check is not JSON: {e}"))?;
    let wrong = |name: &str| format!("its check's {name} is of the wrong type");
    let field = |name: &str| v.get(name).filter(|f| !f.is_null());
    if field("confidence").is_some_and(|c| !c.is_number()) {
        return Err(wrong("confidence"));
    }
    let mut s = read_suitability(&v).map_err(|e| format!("its check is invalid: {e}"))?;
    s.condensed_from_chars = field("condensed_from_chars")
        .map(|f| {
            f.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| wrong("condensed_from_chars"))
        })
        .transpose()?;
    s.condensation_status = field("condensation_status")
        .map(|f| {
            f.as_str()
                .map(str::to_string)
                .ok_or_else(|| wrong("condensation_status"))
        })
        .transpose()?;
    s.notes = match field("notes") {
        Some(n) => serde_json::from_value(n.clone()).map_err(|_| wrong("notes"))?,
        None => Vec::new(),
    };
    Ok(s)
}

/// One part row read back.
fn part_from(r: &Row) -> Result<StoredPrismaPart, String> {
    let id = r.get_i64("id").map_err(|e| e.to_string())?;
    let at = |e: String| format!("part {id}: {e}");
    let text = |name: &str| optional(r, name, text_of).map_err(at);
    Ok(StoredPrismaPart {
        id,
        kind: r.get_str("kind").map_err(|e| e.to_string())?.to_string(),
        item_ids: from_json(r.get_str("item_ids_json").map_err(|e| e.to_string())?).map_err(at)?,
        status: r.get_str("status").map_err(|e| e.to_string())?.to_string(),
        condensed_from_chars: optional(r, "condensed_from_chars", Value::as_i64)
            .map_err(at)?
            .map(|n| usize::try_from(n).map_err(|_| at("a negative length".into())))
            .transpose()?,
        condensation_status: text("condensation_status")?,
        notes: from_json(r.get_str("notes_json").map_err(|e| e.to_string())?).map_err(at)?,
        unasked: from_json(r.get_str("unasked_json").map_err(|e| e.to_string())?).map_err(at)?,
        error_kind: text("error_kind")?,
        error_message: text("error_message")?,
    })
}

/// A stored judgement read back by its name.
fn judgement_of(name: &str) -> Result<Judgement, String> {
    Judgement::parse(name).ok_or_else(|| format!("unknown judgement {name:?}"))
}

/// One item row and its citations, each span checked against `text` and its
/// wording read from it.
fn item_from(db: &mut dyn Db, text: &CanonicalText, r: &Row) -> Result<StoredPrismaItem, String> {
    let str_of = |name: &str| r.get_str(name).map_err(|e| e.to_string());
    let int_of = |name: &str| r.get_i64(name).map_err(|e| e.to_string());
    let stored_id = str_of("item_id")?;
    let id = checklist_item(stored_id)
        .ok_or_else(|| format!("{stored_id:?} is not a checklist item"))?
        .id;
    let at = |e: String| format!("item {id}: {e}");
    let rows = fetch_all(
        db,
        "SELECT span_start, span_end, similarity, exact FROM prisma_citations
         WHERE prisma_item_id = ? ORDER BY position",
        &[Value::from(int_of("id")?)],
    )
    .map_err(|e| e.to_string())?;
    let citations = rows
        .iter()
        .map(|c| citation_from(text, c).map_err(at))
        .collect::<Result<_, _>>()?;
    let dropped = int_of("quotes_dropped")?;
    Ok(StoredPrismaItem {
        part_id: int_of("part_id")?,
        assessment: ItemAssessment {
            id,
            model_judgement: judgement_of(str_of("model_judgement")?).map_err(at)?,
            judgement: judgement_of(str_of("judgement")?).map_err(at)?,
            unbacked: int_of("unbacked")? != 0,
            explanation: str_of("explanation")?.to_string(),
            citations,
            quotes_not_found: from_json(str_of("quotes_not_found_json")?).map_err(at)?,
            quotes_contradicted: contradicted_from_json(str_of("quotes_contradicted_json")?)
                .map_err(at)?,
            quotes_dropped: usize::try_from(dropped)
                .map_err(|_| at("a negative dropped count".into()))?,
        },
        condensed: int_of("condensed")? != 0,
        first_judgement: optional(r, "first_judgement", text_of)
            .map_err(at)?
            .map(|j| judgement_of(&j).map_err(at))
            .transpose()?,
        recheck_part_id: optional(r, "recheck_part_id", Value::as_i64).map_err(at)?,
    })
}

/// The paper and canonical text run `run_id` is on, or `None` if there is no
/// such run.
pub fn run_text(db: &mut dyn Db, run_id: i64) -> Result<Option<(PaperId, i64)>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT paper_id, canonical_id FROM prisma_runs WHERE id = ?",
        &[Value::from(run_id)],
    )?;
    Ok(match row {
        Some(r) => Some((
            PaperId(r.get_str("paper_id")?.to_string()),
            r.get_i64("canonical_id")?,
        )),
        None => None,
    })
}

/// The newest run on canonical text `canonical_id` of `paper`, complete or not.
pub fn latest_run(
    db: &mut dyn Db,
    paper: &PaperId,
    canonical_id: i64,
) -> Result<Option<i64>, StoreError> {
    let row = fetch_one(
        db,
        "SELECT max(id) AS id FROM prisma_runs WHERE paper_id = ? AND canonical_id = ?",
        &[Value::from(paper.as_str()), Value::from(canonical_id)],
    )?;
    Ok(match row {
        Some(r) => r.get("id")?.as_i64(),
        None => None,
    })
}

/// Every run on `paper`, newest first.
pub fn runs(db: &mut dyn Db, paper: &PaperId) -> Result<Vec<PrismaRunSummary>, StoreError> {
    let rows = fetch_all(
        db,
        "SELECT id, canonical_id, model, created_at, status FROM prisma_runs
         WHERE paper_id = ? ORDER BY id DESC",
        &[Value::from(paper.as_str())],
    )?;
    rows.iter()
        .map(|r| {
            Ok(PrismaRunSummary {
                id: r.get_i64("id")?,
                canonical_id: r.get_i64("canonical_id")?,
                model: r.get_str("model")?.to_string(),
                created_at: r.get_str("created_at")?.to_string(),
                status: r.get_str("status")?.to_string(),
            })
        })
        .collect()
}

/// Whether a PRISMA run on canonical text `canonical_id` holds a citation
/// (a re-check's own row included): whether spans point into that text, so
/// it must not be re-derived or replaced. A run with nothing cited is no
/// dependent.
pub fn has_citations(db: &mut dyn Db, canonical_id: i64) -> Result<bool, StoreError> {
    Ok(fetch_one(
        db,
        "SELECT 1 AS one FROM prisma_citations c JOIN prisma_items i ON i.id = c.prisma_item_id
         JOIN prisma_parts p ON p.id = i.part_id JOIN prisma_runs r ON r.id = p.run_id
         WHERE r.canonical_id = ? LIMIT 1",
        &[Value::from(canonical_id)],
    )?
    .is_some())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::pico_store::tests::{cite, count, paper_with_text};
    use crate::store::open_memory_store;
    use bmlib::citations::PassageContradiction;
    use bmlib::quality::{PicoCitation, PicoContradictedQuote};
    use rusqlite::Connection;

    /// A check with verdict `is_suitable`, every field filled so a round trip
    /// shows each one.
    fn check(is_suitable: bool) -> PrismaCheck {
        PrismaCheck {
            suitability: Suitability {
                is_systematic_review: is_suitable,
                is_meta_analysis: false,
                is_suitable,
                document_type: "scoping review".into(),
                variant: Some("scoping review".into()),
                rationale: "Because.".into(),
                confidence: 0.75,
                condensed_from_chars: Some(1234),
                condensation_status: Some("completed".into()),
                notes: vec!["a check note".into()],
            },
            from_abstract: true,
        }
    }

    /// A plan of `parts`, each a list of checklist ids.
    fn plan(parts: &[&[&'static str]]) -> Vec<PartPlan> {
        parts
            .iter()
            .map(|ids| PartPlan {
                item_ids: ids.to_vec(),
            })
            .collect()
    }

    /// Item `id` judged `judgement`, backed by `citations`.
    fn assessed(
        id: &'static str,
        judgement: Judgement,
        citations: Vec<PicoCitation>,
    ) -> ItemAssessment {
        ItemAssessment {
            id,
            model_judgement: judgement,
            judgement,
            unbacked: false,
            explanation: format!("{id} explained"),
            citations,
            quotes_not_found: vec![],
            quotes_contradicted: vec![],
            quotes_dropped: 0,
        }
    }

    /// A part's assessment of `items`, sent whole, with `notes`.
    fn assessment(items: Vec<ItemAssessment>, notes: &[&str]) -> PartAssessment {
        PartAssessment {
            items,
            missing: vec![],
            unasked: vec![],
            condensed_from_chars: None,
            condensation_status: None,
            notes: notes.iter().map(|n| n.to_string()).collect(),
        }
    }

    /// `a` as if judged from a condensed paper.
    pub(crate) fn as_condensed(a: &PartAssessment) -> PartAssessment {
        let mut a = a.clone();
        a.condensed_from_chars.get_or_insert(TEST_CONDENSED_FROM);
        a
    }

    /// The paper length [`as_condensed`] records.
    const TEST_CONDENSED_FROM: usize = 1000;

    /// A run with a passed check and `parts` as its plan, by model "m".
    fn new_run(c: &mut Connection, p: &PaperId, cid: i64, parts: &[&[&'static str]]) -> i64 {
        create_run(c, p, cid, "m", Some(&check(true)), false, &plan(parts)).unwrap()
    }

    /// The ids of the run's pending parts.
    fn pending_ids(c: &mut Connection, run_id: i64) -> Vec<i64> {
        pending_parts(c, run_id)
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect()
    }

    /// The stored run `run_id`.
    fn stored(c: &mut Connection, run_id: i64) -> StoredPrismaRun {
        run(c, run_id).unwrap().unwrap()
    }

    /// Two parts saved, then the run reads back with every item, citation
    /// and note as stored, the citations' text the paper's.
    #[test]
    fn run_round_trips() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["1", "2"], &["3"]]);
        let [first, second] = pending_ids(&mut c, id)[..] else {
            panic!("not two parts");
        };
        let reported = assessed(
            "1",
            Judgement::Reported,
            vec![
                cite(&mut c, &p, cid, "Para two."),
                cite(&mut c, &p, cid, "Para"),
            ],
        );
        let mut unbacked = assessed("2", Judgement::NotReported, vec![]);
        unbacked.model_judgement = Judgement::Reported;
        unbacked.unbacked = true;
        unbacked.quotes_not_found = vec!["Nowhere.".into()];
        unbacked.quotes_contradicted = vec![PicoContradictedQuote {
            quote: "Para three.".into(),
            contradiction: PassageContradiction::Number,
        }];
        unbacked.quotes_dropped = 1;
        let mut condensed = assessment(vec![unbacked.clone(), reported.clone()], &["n1"]);
        condensed.condensed_from_chars = Some(500);
        condensed.condensation_status = Some("completed".into());
        save_part(&mut c, first, &as_condensed(&condensed)).unwrap();
        let elsewhere = assessed(
            "3",
            Judgement::ReportedElsewhere,
            vec![cite(&mut c, &p, cid, "Para one.")],
        );
        save_part(
            &mut c,
            second,
            &assessment(vec![elsewhere.clone()], &["n2"]),
        )
        .unwrap();
        assert!(finish_run(&mut c, id).unwrap());

        let got = stored(&mut c, id);
        assert_eq!((got.id, got.canonical_id), (id, cid));
        assert_eq!(got.model, "m");
        assert!(!got.created_at.is_empty());
        assert_eq!(got.check, Some(check(true)));
        assert!(!got.forced);
        assert_eq!(got.variant.as_deref(), Some("scoping review"));
        assert_eq!(got.status, RUN_COMPLETE);
        assert_eq!(
            got.text,
            canonical_by_id(&mut c, &p, cid).unwrap().unwrap().text
        );
        let shown: Vec<(&ItemAssessment, bool)> = got
            .items
            .iter()
            .map(|i| (&i.assessment, i.condensed))
            .collect();
        assert_eq!(
            shown,
            vec![(&reported, true), (&unbacked, true), (&elsewhere, false)],
            "checklist order"
        );
        assert_eq!(got.items[0].assessment.citations[0].text, "Para two.");
        assert_eq!(got.items[0].part_id, first);
        assert!(got.items.iter().all(|i| i.first_judgement.is_none()));
        let parts: Vec<_> = got
            .parts
            .iter()
            .map(|p| {
                (
                    p.id,
                    p.kind.as_str(),
                    p.status.as_str(),
                    p.condensed_from_chars,
                    p.notes.clone(),
                )
            })
            .collect();
        assert_eq!(
            parts,
            vec![
                (
                    first,
                    KIND_PLAN,
                    PART_DONE,
                    Some(500),
                    vec!["n1".to_string()]
                ),
                (second, KIND_PLAN, PART_DONE, None, vec!["n2".to_string()]),
            ]
        );
        assert_eq!(
            got.parts[0].condensation_status.as_deref(),
            Some("completed")
        );
        assert_eq!(run(&mut c, id + 1).unwrap(), None);
        assert_eq!(run_text(&mut c, id).unwrap(), Some((p.clone(), cid)));
        assert_eq!(run_text(&mut c, id + 1).unwrap(), None);
    }

    /// The plan is stored when the run starts, one pending part per entry in
    /// order, and the run is incomplete with nothing assessed.
    #[test]
    fn create_stores_plan_as_pending() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = create_run(
            &mut c,
            &p,
            cid,
            "m",
            None,
            true,
            &plan(&[&["1", "2"], &["3"], &["13a", "13b"]]),
        )
        .unwrap();
        let pending = pending_parts(&mut c, id).unwrap();
        let shape: Vec<(&str, Vec<String>)> = pending
            .iter()
            .map(|p| (p.kind.as_str(), p.item_ids.clone()))
            .collect();
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            shape,
            vec![
                (KIND_PLAN, ids(&["1", "2"])),
                (KIND_PLAN, ids(&["3"])),
                (KIND_PLAN, ids(&["13a", "13b"])),
            ]
        );
        assert!(pending.windows(2).all(|w| w[0].id < w[1].id));
        let got = stored(&mut c, id);
        assert_eq!(got.status, RUN_INCOMPLETE);
        assert!(got.forced);
        assert_eq!((got.check, got.variant), (None, None));
        assert!(got.items.is_empty());
        assert!(got.parts.iter().all(|p| p.status == PART_PENDING));
    }

    /// A run with no plan, an empty part, an unknown item, an item in two
    /// parts, or a check that
    /// does not allow it (failed and not forced, or none and not forced) is
    /// refused and nothing is written.
    #[test]
    fn create_refuses_bad_runs() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let good = plan(&[&["1"]]);
        let cases: Vec<(Option<PrismaCheck>, bool, Vec<PartPlan>)> = vec![
            (Some(check(true)), false, vec![]),
            (Some(check(true)), false, plan(&[&[]])),
            (Some(check(true)), false, plan(&[&["99z"]])),
            (Some(check(false)), false, good.clone()),
            (None, false, good.clone()),
            (Some(check(true)), false, plan(&[&["1"], &["2", "1"]])),
        ];
        for (chk, forced, parts) in cases {
            assert!(matches!(
                create_run(&mut c, &p, cid, "m", chk.as_ref(), forced, &parts),
                Err(StoreError::Corrupt(_))
            ));
        }
        assert!(matches!(
            create_run(&mut c, &p, cid + 1, "m", None, true, &good),
            Err(StoreError::Corrupt(_))
        ));
        assert_eq!(count(&mut c, "prisma_runs"), 0);
    }

    /// A not-suitable run stores its check, no parts, and is complete; a
    /// passed check cannot be stored as one.
    #[test]
    fn not_suitable_has_no_parts() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = save_not_suitable(&mut c, &p, cid, "m", &check(false)).unwrap();
        let got = stored(&mut c, id);
        assert_eq!(got.check, Some(check(false)));
        assert_eq!(got.status, RUN_COMPLETE);
        assert_eq!(got.variant.as_deref(), Some("scoping review"));
        assert!(got.parts.is_empty() && got.items.is_empty());
        assert!(matches!(
            save_not_suitable(&mut c, &p, cid, "m", &check(true)),
            Err(StoreError::Corrupt(_))
        ));
        assert_eq!(count(&mut c, "prisma_runs"), 1);
    }

    /// A split part is marked `split` and two pending halves follow it, in
    /// order; a split that does not cover the part exactly is refused.
    #[test]
    fn split_replaces_part_with_two_halves() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["1", "2", "3"], &["4"]]);
        let first = pending_ids(&mut c, id)[0];
        for bad in [[&["1"][..], &["2"][..]], [&["2", "3"][..], &["1"][..]]] {
            assert!(matches!(
                split_part(&mut c, first, bad),
                Err(StoreError::Corrupt(_))
            ));
        }
        split_part(&mut c, first, [&["1"], &["2", "3"]]).unwrap();
        let pending = pending_parts(&mut c, id).unwrap();
        let shape: Vec<(&str, Vec<String>)> = pending
            .iter()
            .map(|p| (p.kind.as_str(), p.item_ids.clone()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (KIND_PLAN, vec!["4".to_string()]),
                (KIND_HALF, vec!["1".to_string()]),
                (KIND_HALF, vec!["2".to_string(), "3".to_string()]),
            ]
        );
        let got = stored(&mut c, id);
        assert_eq!(got.parts[0].status, PART_SPLIT);
        assert!(matches!(
            split_part(&mut c, first, [&["1"], &["2", "3"]]),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// A `missing` part cut off is halved into `missing` parts, so an item
    /// a half still leaves out fails at once: its one re-ask is spent.
    #[test]
    fn halved_missing_part_stays_missing() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["3", "4"]]);
        let part = pending_ids(&mut c, id)[0];
        let mut a = assessment(vec![], &[]);
        a.missing = vec!["3", "4"];
        save_part(&mut c, part, &a).unwrap();
        let reask = pending_ids(&mut c, id)[0];
        split_part(&mut c, reask, [&["3"], &["4"]]).unwrap();
        let pending = pending_parts(&mut c, id).unwrap();
        assert!(
            pending.iter().all(|p| p.kind == KIND_MISSING),
            "{pending:?}"
        );
        let mut gap = assessment(vec![], &[]);
        gap.missing = vec!["3"];
        save_part(&mut c, pending[0].id, &gap).unwrap();
        let last = part_shapes(&mut c, id).pop().unwrap();
        assert_eq!(
            last,
            (
                KIND_MISSING.into(),
                PART_FAILED.into(),
                owned(&["3"]),
                Some(ERROR_MISSING_ITEMS.into()),
                Some("The model's reply still left out item(s) 3.".into()),
            )
        );
        assert_eq!(pending_ids(&mut c, id), vec![pending[1].id]);
    }

    /// A condensed part's key items shown `not_reported` (an unbacked one
    /// included) each get a pending `recheck` part in the same save, in the
    /// part's order; a non-key item gets none.
    #[test]
    fn condensed_key_gap_adds_recheck() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["6", "7", "11", "13d"]]);
        let part = pending_ids(&mut c, id)[0];
        let mut unbacked = assessed("11", Judgement::NotReported, vec![]);
        unbacked.model_judgement = Judgement::Reported;
        unbacked.unbacked = true;
        let items = vec![
            assessed("6", Judgement::NotReported, vec![]),
            assessed("7", Judgement::NotReported, vec![]),
            unbacked,
            assessed(
                "13d",
                Judgement::Reported,
                vec![cite(&mut c, &p, cid, "Para")],
            ),
        ];
        save_part(&mut c, part, &as_condensed(&assessment(items, &[]))).unwrap();
        let pending = pending_parts(&mut c, id).unwrap();
        let shape: Vec<(&str, Vec<String>)> = pending
            .iter()
            .map(|p| (p.kind.as_str(), p.item_ids.clone()))
            .collect();
        assert_eq!(
            shape,
            vec![
                (KIND_RECHECK, owned(&["7"])),
                (KIND_RECHECK, owned(&["11"]))
            ]
        );
        assert!(!finish_run(&mut c, id).unwrap());
    }

    /// A part sent whole needs no re-check, whatever it judged.
    #[test]
    fn whole_paper_adds_no_recheck() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["7", "11"]]);
        let part = pending_ids(&mut c, id)[0];
        let items = vec![
            assessed("7", Judgement::NotReported, vec![]),
            assessed("11", Judgement::NotReported, vec![]),
        ];
        save_part(&mut c, part, &assessment(items, &[])).unwrap();
        assert!(pending_parts(&mut c, id).unwrap().is_empty());
        assert!(finish_run(&mut c, id).unwrap());
    }

    /// Saving a re-check applies it: its answer replaces the shown row's
    /// (judgement, explanation, citations), the first judgement is kept, even
    /// over a second re-check, the re-check's own row stays stored, and a
    /// re-check never adds another.
    #[test]
    fn recheck_save_applies_it() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["7"]]);
        let part = pending_ids(&mut c, id)[0];
        let first = assessed(
            "7",
            Judgement::NotReported,
            vec![cite(&mut c, &p, cid, "Para")],
        );
        save_part(&mut c, part, &as_condensed(&assessment(vec![first], &[]))).unwrap();
        let [recheck] = pending_ids(&mut c, id)[..] else {
            panic!("not one re-check");
        };
        let mut found = assessed(
            "7",
            Judgement::Reported,
            vec![cite(&mut c, &p, cid, "Para two.")],
        );
        found.explanation = "found".into();
        let mut a = assessment(vec![found.clone()], &[]);
        a.condensed_from_chars = Some(300);
        save_part(&mut c, recheck, &as_condensed(&a)).unwrap();
        assert!(finish_run(&mut c, id).unwrap());

        let got = stored(&mut c, id);
        assert_eq!(got.items.len(), 1, "one shown row per item");
        let shown = &got.items[0];
        assert_eq!(shown.part_id, part);
        assert_eq!(shown.assessment, found);
        assert_eq!(shown.assessment.citations[0].text, "Para two.");
        assert_eq!(shown.first_judgement, Some(Judgement::NotReported));
        assert_eq!(shown.recheck_part_id, Some(recheck));
        assert!(shown.condensed);
        assert_eq!(
            count(&mut c, "prisma_items"),
            2,
            "the re-check's row is kept"
        );
        assert_eq!(count(&mut c, "prisma_citations"), 2);
        assert_eq!(got.parts[1].kind, KIND_RECHECK);
        assert_eq!(got.parts[1].condensed_from_chars, Some(300));

        // A second re-check cannot arise in a run; stored by hand, it still
        // keeps the first judgement, and its gap adds no third.
        let again = insert_part(&mut c, id, KIND_RECHECK, &["7"]).unwrap();
        let gap = assessed("7", Judgement::NotReported, vec![]);
        save_part(
            &mut c,
            again,
            &as_condensed(&assessment(vec![gap.clone()], &[])),
        )
        .unwrap();
        assert!(pending_parts(&mut c, id).unwrap().is_empty());
        let shown = stored(&mut c, id).items.remove(0);
        assert_eq!(shown.assessment, gap);
        assert_eq!(shown.first_judgement, Some(Judgement::NotReported));
        assert_eq!(shown.recheck_part_id, Some(again));
    }

    /// The judgement, explanation and citation starts of part `part_id`'s
    /// row for item `item_id`, read straight from the tables.
    fn row_answer(c: &mut Connection, part_id: i64, item_id: &str) -> (String, String, Vec<i64>) {
        let row = fetch_one(
            c,
            "SELECT id, judgement, explanation FROM prisma_items WHERE part_id = ? AND item_id = ?",
            &[Value::from(part_id), Value::from(item_id)],
        )
        .unwrap()
        .unwrap();
        let starts = fetch_all(
            c,
            "SELECT span_start FROM prisma_citations WHERE prisma_item_id = ? ORDER BY position",
            &[Value::from(row.get_i64("id").unwrap())],
        )
        .unwrap()
        .iter()
        .map(|r| r.get_i64("span_start").unwrap())
        .collect();
        (
            row.get_str("judgement").unwrap().to_string(),
            row.get_str("explanation").unwrap().to_string(),
            starts,
        )
    }

    /// A re-check swaps: the shown row takes the re-check's answer and the
    /// re-check's own row takes the first answer, so both stay stored, each
    /// once.
    #[test]
    fn recheck_row_keeps_first_answer() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["7"]]);
        let part = pending_ids(&mut c, id)[0];
        let mut first = assessed(
            "7",
            Judgement::NotReported,
            vec![cite(&mut c, &p, cid, "Para")],
        );
        first.explanation = "first".into();
        save_part(
            &mut c,
            part,
            &as_condensed(&assessment(vec![first.clone()], &[])),
        )
        .unwrap();
        let recheck = pending_ids(&mut c, id)[0];
        let mut second = assessed(
            "7",
            Judgement::Reported,
            vec![cite(&mut c, &p, cid, "Para two.")],
        );
        second.explanation = "second".into();
        save_part(
            &mut c,
            recheck,
            &as_condensed(&assessment(vec![second.clone()], &[])),
        )
        .unwrap();

        let start = |a: &ItemAssessment| vec![i64::try_from(a.citations[0].start).unwrap()];
        assert_eq!(
            row_answer(&mut c, part, "7"),
            ("reported".into(), "second".into(), start(&second))
        );
        assert_eq!(
            row_answer(&mut c, recheck, "7"),
            ("not_reported".into(), "first".into(), start(&first))
        );
        assert_eq!(count(&mut c, "prisma_citations"), 2, "each answer's once");
    }

    /// A re-check of an item no part of the run shows is refused, nothing
    /// written.
    #[test]
    fn recheck_without_shown_row_is_refused() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["7"]]);
        let recheck = insert_part(&mut c, id, KIND_RECHECK, &["7"]).unwrap();
        let a = assessment(vec![assessed("7", Judgement::Reported, vec![])], &[]);
        assert!(matches!(
            save_part(&mut c, recheck, &as_condensed(&a)),
            Err(StoreError::NotFound(_))
        ));
        assert_eq!(count(&mut c, "prisma_items"), 0);
        assert_eq!(pending_ids(&mut c, id).len(), 2);
    }

    /// A run completes only once no part is pending or failed.
    #[test]
    fn finish_needs_no_pending_or_failed() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["1"], &["2"]]);
        let [first, second] = pending_ids(&mut c, id)[..] else {
            panic!("not two parts");
        };
        assert!(!finish_run(&mut c, id).unwrap(), "parts pending");
        let one = assessed("1", Judgement::NotReported, vec![]);
        save_part(&mut c, first, &assessment(vec![one], &[])).unwrap();
        fail_part(&mut c, second, "request_failed", "timed out").unwrap();
        assert!(!finish_run(&mut c, id).unwrap(), "a part failed");
        assert_eq!(stored(&mut c, id).status, RUN_INCOMPLETE);
        assert_eq!(reset_failed(&mut c, id).unwrap(), 1);
        let two = assessed("2", Judgement::NotReported, vec![]);
        save_part(&mut c, second, &assessment(vec![two], &[])).unwrap();
        assert!(finish_run(&mut c, id).unwrap());
        assert_eq!(stored(&mut c, id).status, RUN_COMPLETE);
        assert!(matches!(
            finish_run(&mut c, id + 1),
            Err(StoreError::NotFound(_))
        ));
    }

    /// Continue's first step: failed parts are pending again, each keeping
    /// its error until it is saved or split, and the count says how many.
    #[test]
    fn reset_failed_makes_failed_pending() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["1"], &["2"], &["3"]]);
        let ids = pending_ids(&mut c, id);
        fail_part(&mut c, ids[0], "truncated", "cut off").unwrap();
        fail_part(&mut c, ids[2], "request_failed", "timed out").unwrap();
        assert_eq!(pending_ids(&mut c, id), vec![ids[1]]);
        let failed = stored(&mut c, id).parts.remove(0);
        assert_eq!(
            (
                failed.status.as_str(),
                failed.error_kind.as_deref(),
                failed.error_message.as_deref()
            ),
            (PART_FAILED, Some("truncated"), Some("cut off"))
        );
        assert_eq!(reset_failed(&mut c, id).unwrap(), 2);
        assert_eq!(pending_ids(&mut c, id), ids);
        let parts = stored(&mut c, id).parts;
        assert_eq!(
            parts
                .iter()
                .map(|p| (p.error_kind.as_deref(), p.error_message.as_deref()))
                .collect::<Vec<_>>(),
            [
                (Some("truncated"), Some("cut off")),
                (None, None),
                (Some("request_failed"), Some("timed out"))
            ]
        );
        assert_eq!(reset_failed(&mut c, id).unwrap(), 0);
        let reported = assessed("1", Judgement::Reported, vec![]);
        save_part(&mut c, ids[0], &assessment(vec![reported], &[])).unwrap();
        let saved = stored(&mut c, id).parts.remove(0);
        assert_eq!(
            (saved.status.as_str(), saved.error_kind, saved.error_message),
            (PART_DONE, None, None),
            "a saved part's earlier error is cleared"
        );
    }

    /// A part that holds items (it is done) cannot be failed, split or saved
    /// again: a failed part never holds items.
    #[test]
    fn only_a_pending_part_changes() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["1"]]);
        let part = pending_ids(&mut c, id)[0];
        let a = assessment(vec![assessed("1", Judgement::NotReported, vec![])], &[]);
        save_part(&mut c, part, &a).unwrap();
        assert!(matches!(
            fail_part(&mut c, part, "k", "m"),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            save_part(&mut c, part, &a),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            split_part(&mut c, part, [&["1"], &[]]),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            fail_part(&mut c, part + 1, "k", "m"),
            Err(StoreError::NotFound(_))
        ));
        assert_eq!(stored(&mut c, id).parts[0].status, PART_DONE);
        assert_eq!(count(&mut c, "prisma_items"), 1);
    }

    /// An item the part was not asked, an item already shown by another
    /// part, or a citation outside the text is refused and nothing written.
    #[test]
    fn save_part_checks_items_and_spans() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["1"], &["2"]]);
        let [first, second] = pending_ids(&mut c, id)[..] else {
            panic!("not two parts");
        };
        let mut outside = cite(&mut c, &p, cid, "Para");
        outside.end = 9999;
        let bad = [
            assessed("2", Judgement::NotReported, vec![]),
            assessed("1", Judgement::Reported, vec![outside]),
        ];
        for item in bad {
            assert!(matches!(
                save_part(&mut c, first, &assessment(vec![item], &[])),
                Err(StoreError::Corrupt(_))
            ));
        }
        assert_eq!(count(&mut c, "prisma_items"), 0);
        let one = assessed("1", Judgement::NotReported, vec![]);
        save_part(&mut c, first, &assessment(vec![one.clone()], &[])).unwrap();
        let missing = insert_part(&mut c, id, KIND_MISSING, &["1"]).unwrap();
        assert!(matches!(
            save_part(&mut c, missing, &assessment(vec![one], &[])),
            Err(StoreError::Corrupt(_))
        ));
        assert_eq!(pending_ids(&mut c, id), vec![second, missing]);
    }

    /// A part as the follow-up tests compare it: kind, status, items, error
    /// kind and error message.
    type PartShape = (String, String, Vec<String>, Option<String>, Option<String>);

    /// The shape of every part of run `run_id`, in id order.
    fn part_shapes(c: &mut Connection, run_id: i64) -> Vec<PartShape> {
        stored(c, run_id)
            .parts
            .into_iter()
            .map(|p| (p.kind, p.status, p.item_ids, p.error_kind, p.error_message))
            .collect()
    }

    /// `ids` as owned strings.
    fn owned(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    /// A reply that leaves items out stores what it has and, in the same
    /// save, a pending `missing` part for exactly the rest, in asked order:
    /// the run cannot complete with a gap.
    #[test]
    fn missing_item_gets_a_pending_part() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["5", "6", "7"]]);
        let part = pending_ids(&mut c, id)[0];
        let mut a = assessment(vec![assessed("6", Judgement::NotReported, vec![])], &[]);
        a.missing = vec!["7", "5"];
        save_part(&mut c, part, &a).unwrap();
        let pending = pending_parts(&mut c, id).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].kind, KIND_MISSING);
        assert_eq!(pending[0].item_ids, owned(&["5", "7"]));
        assert!(!finish_run(&mut c, id).unwrap());
        assert_eq!(stored(&mut c, id).items.len(), 1);
    }

    /// On a `missing` part, the found items are saved and the rest go to a
    /// `missing` part failed at once, naming them; Continue's reset makes
    /// it pending, and answering it completes the run.
    #[test]
    fn still_missing_fails_follow_up() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["7", "8", "13d"]]);
        let part = pending_ids(&mut c, id)[0];
        let mut a = assessment(vec![], &[]);
        a.missing = vec!["7", "8", "13d"];
        save_part(&mut c, part, &a).unwrap();
        let reask = pending_ids(&mut c, id)[0];
        let mut again = assessment(vec![assessed("8", Judgement::Reported, vec![])], &[]);
        again.missing = vec!["7", "13d"];
        save_part(&mut c, reask, &again).unwrap();
        assert!(pending_parts(&mut c, id).unwrap().is_empty());
        assert_eq!(
            part_shapes(&mut c, id)[2],
            (
                KIND_MISSING.into(),
                PART_FAILED.into(),
                owned(&["7", "13d"]),
                Some(ERROR_MISSING_ITEMS.into()),
                Some("The model's reply still left out item(s) 7, 13d.".into()),
            )
        );
        assert!(!finish_run(&mut c, id).unwrap(), "a failed follow-up");
        let shown: Vec<&str> = stored(&mut c, id)
            .items
            .iter()
            .map(|i| i.assessment.id)
            .collect();
        assert_eq!(shown, vec!["8"]);

        assert_eq!(reset_failed(&mut c, id).unwrap(), 1);
        let pending = pending_parts(&mut c, id).unwrap();
        assert_eq!(pending[0].item_ids, owned(&["7", "13d"]));
        let last = assessment(
            vec![
                assessed("7", Judgement::NotReported, vec![]),
                assessed("13d", Judgement::NotReported, vec![]),
            ],
            &[],
        );
        save_part(&mut c, pending[0].id, &last).unwrap();
        assert!(finish_run(&mut c, id).unwrap());
    }

    /// A re-check whose reply leaves its item out stores a `recheck` part
    /// failed at once, naming it, and the shown row is untouched.
    #[test]
    fn missing_on_recheck_fails_recheck() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["7"]]);
        let part = pending_ids(&mut c, id)[0];
        let first = assessed("7", Judgement::NotReported, vec![]);
        save_part(
            &mut c,
            part,
            &as_condensed(&assessment(vec![first.clone()], &[])),
        )
        .unwrap();
        let [recheck] = pending_ids(&mut c, id)[..] else {
            panic!("not one re-check");
        };
        let mut a = assessment(vec![], &[]);
        a.missing = vec!["7"];
        save_part(&mut c, recheck, &as_condensed(&a)).unwrap();
        let shapes = part_shapes(&mut c, id);
        assert_eq!(shapes[1].1, PART_DONE);
        assert_eq!(
            shapes[2],
            (
                KIND_RECHECK.into(),
                PART_FAILED.into(),
                owned(&["7"]),
                Some(ERROR_MISSING_ITEMS.into()),
                Some("The model's reply still left out item(s) 7.".into()),
            )
        );
        assert!(!finish_run(&mut c, id).unwrap());
        assert_eq!(stored(&mut c, id).items[0].assessment, first);
        assert_eq!(reset_failed(&mut c, id).unwrap(), 1);
        assert_eq!(pending_parts(&mut c, id).unwrap()[0].kind, KIND_RECHECK);
    }

    /// A reply whose items and missing ids are not exactly the part's asks
    /// (one in neither, one not asked, one in both) is refused, and nothing
    /// is written.
    #[test]
    fn uncovered_ask_is_refused() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["1", "2"], &["3"]]);
        let part = pending_ids(&mut c, id)[0];
        let one = || assessed("1", Judgement::NotReported, vec![]);
        let cases: Vec<(Vec<ItemAssessment>, Vec<&'static str>)> = vec![
            (vec![one()], vec![]),
            (vec![], vec!["1"]),
            (vec![one()], vec!["2", "3"]),
            (vec![one()], vec!["1", "2"]),
        ];
        for (items, missing) in cases {
            let mut a = assessment(items, &[]);
            a.missing = missing;
            assert!(matches!(
                save_part(&mut c, part, &a),
                Err(StoreError::Corrupt(_))
            ));
        }
        assert_eq!(count(&mut c, "prisma_items"), 0);
        assert_eq!(count(&mut c, "prisma_parts"), 2);
        assert_eq!(pending_ids(&mut c, id).len(), 2);
    }

    /// The latest run is the newest on that text; the list holds every run,
    /// newest first, with its status.
    #[test]
    fn latest_is_newest_on_that_text() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let first = save_not_suitable(&mut c, &p, cid, "m", &check(false)).unwrap();
        let second = new_run(&mut c, &p, cid, &[&["1"]]);
        assert_eq!(latest_run(&mut c, &p, cid).unwrap(), Some(second));
        assert_eq!(latest_run(&mut c, &p, cid + 1).unwrap(), None);
        let list = runs(&mut c, &p).unwrap();
        let shape: Vec<(i64, &str)> = list.iter().map(|s| (s.id, s.status.as_str())).collect();
        assert_eq!(shape, vec![(second, RUN_INCOMPLETE), (first, RUN_COMPLETE)]);
        assert_eq!((list[0].canonical_id, list[0].model.as_str()), (cid, "m"));
        assert!(!list[0].created_at.is_empty());
    }

    /// Deleting the paper deletes its runs, parts, items and citations.
    #[test]
    fn deleting_paper_cascades() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["1"], &["2"]]);
        let part = pending_ids(&mut c, id)[0];
        let one = assessed(
            "1",
            Judgement::Reported,
            vec![cite(&mut c, &p, cid, "Para")],
        );
        save_part(&mut c, part, &assessment(vec![one], &[])).unwrap();
        let recheck = insert_part(&mut c, id, KIND_RECHECK, &["1"]).unwrap();
        let again = assessed(
            "1",
            Judgement::Reported,
            vec![cite(&mut c, &p, cid, "Para")],
        );
        save_part(&mut c, recheck, &assessment(vec![again], &[])).unwrap();
        save_not_suitable(&mut c, &p, cid, "m", &check(false)).unwrap();
        execute(
            &mut c,
            "DELETE FROM papers WHERE id = ?",
            &[Value::from(p.as_str())],
        )
        .unwrap();
        for table in [
            "prisma_runs",
            "prisma_parts",
            "prisma_items",
            "prisma_citations",
        ] {
            assert_eq!(count(&mut c, table), 0, "{table}");
        }
    }

    /// Only a stored citation makes a text a dependent.
    #[test]
    fn has_citations_counts_only_citations() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        save_not_suitable(&mut c, &p, cid, "m", &check(false)).unwrap();
        assert!(!has_citations(&mut c, cid).unwrap());
        let id = new_run(&mut c, &p, cid, &[&["1"], &["2"]]);
        let [first, second] = pending_ids(&mut c, id)[..] else {
            panic!("not two parts");
        };
        let uncited = assessed("1", Judgement::NotReported, vec![]);
        save_part(&mut c, first, &assessment(vec![uncited], &[])).unwrap();
        assert!(!has_citations(&mut c, cid).unwrap());
        let cited = assessed(
            "2",
            Judgement::Reported,
            vec![cite(&mut c, &p, cid, "Para")],
        );
        save_part(&mut c, second, &assessment(vec![cited], &[])).unwrap();
        assert!(has_citations(&mut c, cid).unwrap());
        assert!(!has_citations(&mut c, cid + 1).unwrap());
    }

    /// Rows this code did not write are reported, not shown.
    #[test]
    fn corrupt_rows_reported() {
        let cases: &[(&str, &str)] = &[
            ("unknown item", "UPDATE prisma_items SET item_id = '99z'"),
            ("unknown judgement", "UPDATE prisma_items SET judgement = 'maybe'"),
            (
                "unknown first judgement",
                "UPDATE prisma_items SET first_judgement = 'maybe'",
            ),
            ("span past the end", "UPDATE prisma_citations SET span_end = 9999"),
            ("notes not a list", "UPDATE prisma_parts SET notes_json = '{'"),
            ("unasked not a list", "UPDATE prisma_parts SET unasked_json = '{'"),
            ("item ids not a list", "UPDATE prisma_parts SET item_ids_json = '{'"),
            ("negative dropped count", "UPDATE prisma_items SET quotes_dropped = -1"),
            (
                "check without what it read",
                "UPDATE prisma_runs SET suitability_from_abstract = NULL",
            ),
            (
                "check field of the wrong type",
                "UPDATE prisma_runs SET suitability_json = replace(suitability_json, '\"confidence\":0.75', '\"confidence\":\"high\"')",
            ),
            (
                "two shown rows for one item",
                "INSERT INTO prisma_items (part_id, item_id, model_judgement, judgement, unbacked, explanation,
                   condensed, quotes_not_found_json, quotes_contradicted_json, quotes_dropped)
                 SELECT max(id), '1', 'reported', 'reported', 0, '', 0, '[]', '[]', 0 FROM prisma_parts",
            ),
        ];
        for &(what, sql) in cases {
            let mut c = open_memory_store().unwrap();
            let (p, cid) = paper_with_text(&mut c);
            let id = new_run(&mut c, &p, cid, &[&["1"], &["2"]]);
            let part = pending_ids(&mut c, id)[0];
            let one = assessed(
                "1",
                Judgement::Reported,
                vec![cite(&mut c, &p, cid, "Para")],
            );
            save_part(&mut c, part, &assessment(vec![one], &["n"])).unwrap();
            execute(&mut c, sql, &[]).unwrap();
            assert!(
                matches!(run(&mut c, id), Err(StoreError::Corrupt(_))),
                "{what}"
            );
        }
    }
}
