//! PRISMA 2020 runs on one paper (design 2026-11-18, §7), stored part by
//! part. A run starts with its part plan stored as `pending` parts; each part
//! is stored the moment it finishes, so a run that is stopped or fails keeps
//! what it finished and Continue runs only what is left. A run is on one
//! canonical text, so every stored span points into it.
//!
//! One shown row per checklist item: the row a non-`recheck` part stored. A
//! re-check (§6.5) stores its own row too, kept for audit, and
//! [`apply_recheck`] copies its answer onto the shown row.

use bmlib::db::{execute, fetch_all, fetch_one, transaction_with, Db, Row, Value};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::quality::prisma2020_checklist::{item as checklist_item, CHECKLIST};
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
/// One half of a part whose reply was cut off at its limit.
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
/// A part whose call failed after its retries; it holds no items.
pub const PART_FAILED: &str = "failed";
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
    /// For a failed part, the error's kind.
    pub error_kind: Option<String>,
    /// For a failed part, the error's message.
    pub error_message: Option<String>,
}

/// One shown checklist item of a run.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredPrismaItem {
    /// The (non-`recheck`) part that stored it.
    pub part_id: i64,
    /// The item as shown; every citation's `text` re-read from the paper.
    pub assessment: ItemAssessment,
    /// Whether it was judged from a condensed paper.
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
/// `check`. An empty plan, a part with no items or an unknown item, a text
/// that is not `paper`'s, or a check that does not allow the run is
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
    for part in plan {
        check_ids(&part.item_ids)?;
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
/// and their citations, its condensation and notes, and the part as `done`.
/// `condensed` says whether its paper was condensed. The ids the reply left
/// out (`assessment.missing`) are not stored here: the caller adds a
/// [`KIND_MISSING`] part for them. Refused as [`StoreError::Corrupt`], with
/// nothing written: a part not pending, an item the part did not ask, an
/// item another non-re-check part of the run already shows, or a citation
/// outside the run's text.
pub fn save_part(
    db: &mut dyn Db,
    part_id: i64,
    assessment: &PartAssessment,
    condensed: bool,
) -> Result<(), StoreError> {
    let part = pending_part(db, part_id)?;
    let (paper, canonical_id) = run_text(db, part.run_id)?
        .ok_or_else(|| StoreError::NotFound(format!("PRISMA run {}", part.run_id)))?;
    let text = text_of_paper(db, &paper, canonical_id)?;
    for item in &assessment.items {
        if !part.item_ids.iter().any(|id| id == item.id) {
            return Err(StoreError::Corrupt(format!(
                "PRISMA part {part_id} did not ask item {}",
                item.id
            )));
        }
        if part.kind != KIND_RECHECK && shown_row(db, part.run_id, item.id)?.is_some() {
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
    let notes = to_json(&assessment.notes)?;
    transaction_with(db, |tx| {
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
                                     notes_json = ?
             WHERE id = ?",
            &[
                Value::from(PART_DONE),
                Value::from(assessment.condensed_from_chars),
                Value::from(assessment.condensation_status.clone()),
                Value::from(notes.as_str()),
                Value::from(part_id),
            ],
        )?;
        Ok(())
    })
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
/// pending [`KIND_HALF`] parts asking `halves`, and marks it `split`, in one
/// transaction. The halves, in order, must ask exactly the part's items in
/// its order, each half at least one; otherwise, or for a part not pending,
/// [`StoreError::Corrupt`].
pub fn split_part(db: &mut dyn Db, part_id: i64, halves: [&[&str]; 2]) -> Result<(), StoreError> {
    let part = pending_part(db, part_id)?;
    let joined: Vec<&str> = halves.iter().flat_map(|h| h.iter().copied()).collect();
    if halves.iter().any(|h| h.is_empty()) || joined != part.item_ids {
        return Err(StoreError::Corrupt(format!(
            "halves {halves:?} do not split PRISMA part {part_id}'s items {:?}",
            part.item_ids
        )));
    }
    transaction_with(db, |tx| {
        execute(
            tx,
            "UPDATE prisma_parts SET status = ? WHERE id = ?",
            &[Value::from(PART_SPLIT), Value::from(part_id)],
        )?;
        for half in halves {
            insert_part(tx, part.run_id, KIND_HALF, half)?;
        }
        Ok(())
    })
}

/// Adds a pending part of `kind` to run `run_id`, asking `item_ids`, and
/// returns its id: a [`KIND_MISSING`] part for the items a reply left out, or
/// a [`KIND_RECHECK`] part for one key item (§6.5). Another kind, no items,
/// or an unknown item is [`StoreError::Corrupt`]; a run that does not exist
/// is refused by the database.
pub fn add_part(
    db: &mut dyn Db,
    run_id: i64,
    kind: &str,
    item_ids: &[&str],
) -> Result<i64, StoreError> {
    if kind != KIND_MISSING && kind != KIND_RECHECK {
        return Err(StoreError::Corrupt(format!(
            "a part added to a run is {KIND_MISSING:?} or {KIND_RECHECK:?}, not {kind:?}"
        )));
    }
    check_ids(item_ids)?;
    insert_part(db, run_id, kind, item_ids)
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
    pending_part(db, part_id)?;
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

/// Makes run `run_id`'s failed parts pending again, their errors cleared,
/// and returns how many: Continue calls it before running what is pending.
pub fn reset_failed(db: &mut dyn Db, run_id: i64) -> Result<usize, StoreError> {
    let n = execute(
        db,
        "UPDATE prisma_parts SET status = ?, error_kind = NULL, error_message = NULL
         WHERE run_id = ? AND status = ?",
        &[
            Value::from(PART_PENDING),
            Value::from(run_id),
            Value::from(PART_FAILED),
        ],
    )?;
    usize::try_from(n).map_err(|_| StoreError::Corrupt(format!("{n} parts reset")))
}

/// Copies re-check part `recheck_part_id`'s answer for `item_id` onto the
/// item's shown row in run `run_id` (the row its non-re-check part stored),
/// in one transaction: the model's and the shown judgement, `unbacked`, the
/// explanation, the unlocated quotes and the dropped count, and its
/// citations (replaced by copies). The shown judgement before the first
/// re-check is kept in `first_judgement` and never overwritten by a later
/// one; `recheck_part_id` names the latest. Whether the item was judged from
/// a condensation stays the shown row's. The re-check's own row stays stored.
/// No such re-check row, or no shown row, is [`StoreError::NotFound`].
pub fn apply_recheck(
    db: &mut dyn Db,
    run_id: i64,
    item_id: &str,
    recheck_part_id: i64,
) -> Result<(), StoreError> {
    let recheck = fetch_one(
        db,
        "SELECT i.id FROM prisma_items i JOIN prisma_parts p ON p.id = i.part_id
         WHERE p.id = ? AND p.run_id = ? AND p.kind = ? AND i.item_id = ?",
        &[
            Value::from(recheck_part_id),
            Value::from(run_id),
            Value::from(KIND_RECHECK),
            Value::from(item_id),
        ],
    )?
    .ok_or_else(|| {
        StoreError::NotFound(format!(
            "a re-check of item {item_id} in PRISMA part {recheck_part_id} of run {run_id}"
        ))
    })?
    .get_i64("id")?;
    let shown = shown_row(db, run_id, item_id)?.ok_or_else(|| {
        StoreError::NotFound(format!("item {item_id} assessed in PRISMA run {run_id}"))
    })?;
    transaction_with(db, |tx| {
        // Every right-hand side reads the row before the update, so
        // `first_judgement` takes the judgement being replaced.
        execute(
            tx,
            "UPDATE prisma_items SET
               first_judgement = coalesce(first_judgement, judgement),
               recheck_part_id = ?,
               (model_judgement, judgement, unbacked, explanation, quotes_not_found_json,
                quotes_contradicted_json, quotes_dropped) =
               (SELECT model_judgement, judgement, unbacked, explanation, quotes_not_found_json,
                       quotes_contradicted_json, quotes_dropped
                FROM prisma_items WHERE id = ?)
             WHERE id = ?",
            &[
                Value::from(recheck_part_id),
                Value::from(recheck),
                Value::from(shown),
            ],
        )?;
        execute(
            tx,
            "DELETE FROM prisma_citations WHERE prisma_item_id = ?",
            &[Value::from(shown)],
        )?;
        execute(
            tx,
            "INSERT INTO prisma_citations (prisma_item_id, position, span_start, span_end,
                                           similarity, exact)
             SELECT ?, position, span_start, span_end, similarity, exact
             FROM prisma_citations WHERE prisma_item_id = ?",
            &[Value::from(shown), Value::from(recheck)],
        )?;
        Ok(())
    })
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
                notes_json, error_kind, error_message
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
mod tests {
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
            condensed_from_chars: None,
            condensation_status: None,
            notes: notes.iter().map(|n| n.to_string()).collect(),
        }
    }

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
        save_part(&mut c, first, &condensed, true).unwrap();
        let elsewhere = assessed(
            "3",
            Judgement::ReportedElsewhere,
            vec![cite(&mut c, &p, cid, "Para one.")],
        );
        save_part(
            &mut c,
            second,
            &assessment(vec![elsewhere.clone()], &["n2"]),
            false,
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

    /// A run with no plan, an empty part, an unknown item, or a check that
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

    /// The re-check's answer replaces the shown row (judgement, explanation,
    /// citations), the first judgement is kept, even over a second re-check,
    /// and the re-check's own row stays stored.
    #[test]
    fn recheck_keeps_first_judgement() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["7"]]);
        let part = pending_ids(&mut c, id)[0];
        let first = assessed(
            "7",
            Judgement::NotReported,
            vec![cite(&mut c, &p, cid, "Para")],
        );
        save_part(&mut c, part, &assessment(vec![first], &[]), true).unwrap();
        let recheck = add_part(&mut c, id, KIND_RECHECK, &["7"]).unwrap();
        let mut found = assessed(
            "7",
            Judgement::Reported,
            vec![cite(&mut c, &p, cid, "Para two.")],
        );
        found.explanation = "found".into();
        let mut a = assessment(vec![found.clone()], &[]);
        a.condensed_from_chars = Some(300);
        save_part(&mut c, recheck, &a, true).unwrap();
        apply_recheck(&mut c, id, "7", recheck).unwrap();

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

        let again = add_part(&mut c, id, KIND_RECHECK, &["7"]).unwrap();
        let partly = assessed("7", Judgement::PartlyReported, vec![]);
        save_part(&mut c, again, &assessment(vec![partly.clone()], &[]), true).unwrap();
        apply_recheck(&mut c, id, "7", again).unwrap();
        let shown = stored(&mut c, id).items.remove(0);
        assert_eq!(shown.assessment, partly);
        assert_eq!(shown.first_judgement, Some(Judgement::NotReported));
        assert_eq!(shown.recheck_part_id, Some(again));
        assert!(matches!(
            apply_recheck(&mut c, id, "7", part),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            apply_recheck(&mut c, id, "8", again),
            Err(StoreError::NotFound(_))
        ));
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
        save_part(&mut c, first, &assessment(vec![one], &[]), false).unwrap();
        fail_part(&mut c, second, "request_failed", "timed out").unwrap();
        assert!(!finish_run(&mut c, id).unwrap(), "a part failed");
        assert_eq!(stored(&mut c, id).status, RUN_INCOMPLETE);
        assert_eq!(reset_failed(&mut c, id).unwrap(), 1);
        let two = assessed("2", Judgement::NotReported, vec![]);
        save_part(&mut c, second, &assessment(vec![two], &[]), false).unwrap();
        assert!(finish_run(&mut c, id).unwrap());
        assert_eq!(stored(&mut c, id).status, RUN_COMPLETE);
        assert!(matches!(
            finish_run(&mut c, id + 1),
            Err(StoreError::NotFound(_))
        ));
    }

    /// Continue's first step: failed parts are pending again, their error
    /// cleared, and the count says how many.
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
        assert!(parts
            .iter()
            .all(|p| p.error_kind.is_none() && p.error_message.is_none()));
        assert_eq!(reset_failed(&mut c, id).unwrap(), 0);
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
        save_part(&mut c, part, &a, false).unwrap();
        assert!(matches!(
            fail_part(&mut c, part, "k", "m"),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            save_part(&mut c, part, &a, false),
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
                save_part(&mut c, first, &assessment(vec![item], &[]), false),
                Err(StoreError::Corrupt(_))
            ));
        }
        assert_eq!(count(&mut c, "prisma_items"), 0);
        let one = assessed("1", Judgement::NotReported, vec![]);
        save_part(&mut c, first, &assessment(vec![one.clone()], &[]), false).unwrap();
        let missing = add_part(&mut c, id, KIND_MISSING, &["1"]).unwrap();
        assert!(matches!(
            save_part(&mut c, missing, &assessment(vec![one], &[]), false),
            Err(StoreError::Corrupt(_))
        ));
        assert_eq!(pending_ids(&mut c, id), vec![second, missing]);
    }

    /// Only a `missing` or `recheck` part of known items can be added.
    #[test]
    fn add_part_takes_missing_or_recheck() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let id = new_run(&mut c, &p, cid, &[&["1"]]);
        for (kind, ids) in [
            (KIND_PLAN, &["2"][..]),
            (KIND_HALF, &["2"][..]),
            (KIND_MISSING, &[][..]),
            (KIND_RECHECK, &["nope"][..]),
        ] {
            assert!(matches!(
                add_part(&mut c, id, kind, ids),
                Err(StoreError::Corrupt(_))
            ));
        }
        let missing = add_part(&mut c, id, KIND_MISSING, &["2", "3"]).unwrap();
        let pending = pending_parts(&mut c, id).unwrap();
        assert_eq!(pending[1].id, missing);
        assert_eq!(pending[1].kind, KIND_MISSING);
        assert_eq!(pending[1].item_ids, vec!["2".to_string(), "3".to_string()]);
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
        save_part(&mut c, part, &assessment(vec![one], &[]), false).unwrap();
        let recheck = add_part(&mut c, id, KIND_RECHECK, &["1"]).unwrap();
        let again = assessed(
            "1",
            Judgement::Reported,
            vec![cite(&mut c, &p, cid, "Para")],
        );
        save_part(&mut c, recheck, &assessment(vec![again], &[]), false).unwrap();
        apply_recheck(&mut c, id, "1", recheck).unwrap();
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
        save_part(&mut c, first, &assessment(vec![uncited], &[]), false).unwrap();
        assert!(!has_citations(&mut c, cid).unwrap());
        let cited = assessed(
            "2",
            Judgement::Reported,
            vec![cite(&mut c, &p, cid, "Para")],
        );
        save_part(&mut c, second, &assessment(vec![cited], &[]), false).unwrap();
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
            save_part(&mut c, part, &assessment(vec![one], &["n"]), false).unwrap();
            execute(&mut c, sql, &[]).unwrap();
            assert!(
                matches!(run(&mut c, id), Err(StoreError::Corrupt(_))),
                "{what}"
            );
        }
    }
}
