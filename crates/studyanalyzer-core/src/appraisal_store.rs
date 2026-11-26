//! The reader's own blinded RoB 2 appraisal of one paper (design 2026-11-26,
//! §3–§8), stored in tables of its own beside the machine's runs.
//!
//! An appraisal starts on the paper's latest canonical text with one empty
//! version, the *first record* (number 0). [`confirm`] stores the confirmed
//! design and outcomes and readies the paired machine run; [`edit`] writes
//! the reader's answers, marks, ticks, overrides and notes to the editable
//! version; [`save`] freezes the first record. A change after the save goes
//! to a revision (*after reveal 1*, 2, …), copied from the latest version,
//! until [`close_revision`] closes it. A frozen or closed version is never
//! written. A mark is a span only: its wording is re-read from the text.
//! The rows read back, and their shape check, are in
//! [`crate::appraisal_rows`].

use std::collections::BTreeSet;

use bmlib::db::{execute, fetch_one, transaction_with, Db, Value};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::quality::{
    allowed_answers, questions, start_level, study_design_from_str, Answer, Answers, Domain,
    EvidenceLadder, Judgement, ReportedResult, LADDER_VERSION, ROB2_ALGORITHM_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::appraisal::{
    complete, derived, effective, outcome_verdict, RaterDomain, RaterOverall, RaterRecord,
    OUTCOME_DOMAINS,
};
use crate::appraisal_rows::{
    appraisal, check_outcome, check_question, check_scope, check_span, design_covered,
    AppraisalState, StoredAppraisal, StoredVersion, ANSWERS, ANSWER_COLUMNS, DOMAINS,
    DOMAIN_COLUMNS, OVERALLS, OVERALL_COLUMNS,
};
use crate::config::APPRAISAL_NOTE_MAX_CHARS;
use crate::evidence_rows::{ladder_step_view, wire};
use crate::evidence_store::{domain_name, plan_paired, StepStatus, STEP_OUTCOMES};
use crate::store::{last_id, latest_canonical_id, PaperId, StoreError};

/// What the reader confirms in steps 1 and 2: the design, its layout, and
/// the primary outcomes to appraise, as the webview sends them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmInput {
    /// The design's bmlib name (`rct`, `cohort_prospective`, …).
    pub design: String,
    /// A randomised trial's layout (`parallel`, …); `None` for any other design.
    pub rct_layout: Option<String>,
    /// The confirmed outcomes, in the order they are appraised.
    pub outcomes: Vec<ConfirmedOutcomeInput>,
}

/// One confirmed outcome as the reader leaves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ConfirmedOutcomeInput {
    /// The index of the machine's proposed outcome it came from, or `None`
    /// for one the reader added.
    pub from_proposed: Option<usize>,
    /// The outcome's name.
    pub name: String,
    /// When it was measured.
    pub timepoint: Option<String>,
    /// Its reported effect, if any.
    pub result: Option<ReportedResult>,
}

/// One write of the reader to their appraisal, as the webview sends it.
/// `domain` and `outcome` name where it goes: D1 with no outcome, D2–D5 with
/// the index of a confirmed outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppraisalEdit {
    /// The answer to a signalling question; `None` clears it.
    Answer {
        /// The question's domain.
        domain: Domain,
        /// The confirmed outcome, `None` for D1.
        outcome: Option<usize>,
        /// RoB 2's question number (`2.3`).
        question_id: String,
        /// The answer, one of `allowed_answers(question_id)`.
        answer: Option<Answer>,
    },
    /// The sentence the answer rests on; `None` clears it.
    Mark {
        /// The question's domain.
        domain: Domain,
        /// The confirmed outcome, `None` for D1.
        outcome: Option<usize>,
        /// RoB 2's question number.
        question_id: String,
        /// The span in code points of the canonical text (the command
        /// converts the webview's UTF-16 span).
        span: Option<TextSpan>,
    },
    /// The ticks *I also used: protocol / registry / supplement*.
    Sources {
        /// The question's domain.
        domain: Domain,
        /// The confirmed outcome, `None` for D1.
        outcome: Option<usize>,
        /// RoB 2's question number.
        question_id: String,
        /// The trial's protocol was used.
        protocol: bool,
        /// A trial registry entry was used.
        registry: bool,
        /// A supplement was used.
        supplement: bool,
    },
    /// The reader's own verdict on a domain over the derived one; `None`
    /// clears it.
    Override {
        /// The domain.
        domain: Domain,
        /// The confirmed outcome, `None` for D1.
        outcome: Option<usize>,
        /// The verdict.
        judgement: Option<Judgement>,
    },
    /// The answer to the escalation question of one outcome; `None` clears it.
    Escalate {
        /// The confirmed outcome.
        outcome: usize,
        /// Whether the concerns together make the risk of bias high.
        escalate: Option<bool>,
    },
    /// The reader's own overall verdict on one outcome; `None` clears it.
    OverallOverride {
        /// The confirmed outcome.
        outcome: usize,
        /// The verdict.
        judgement: Option<Judgement>,
    },
    /// A free-text note on a domain; `None` clears it.
    Note {
        /// The domain.
        domain: Domain,
        /// The confirmed outcome, `None` for D1.
        outcome: Option<usize>,
        /// The note, at most `APPRAISAL_NOTE_MAX_CHARS` characters.
        note: Option<String>,
    },
}

/// Starts an appraisal of `paper` on its latest canonical text, with an
/// empty first record under the current rules, and returns its id. It is
/// blind only if no run of the paper has been shown (`first_shown_at`). A
/// paper with an open appraisal already is [`StoreError::Invalid`]; one with
/// no text is [`StoreError::NotFound`].
pub fn start_appraisal(db: &mut dyn Db, paper: &PaperId) -> Result<i64, StoreError> {
    transaction_with(db, |tx| {
        let canonical_id = latest_canonical_id(tx, paper)?
            .ok_or_else(|| StoreError::NotFound(format!("paper {}'s full text", paper.as_str())))?;
        if open_appraisal(tx, paper)?.is_some() {
            log::info!(
                "paper {}: a second appraisal refused, one is open",
                paper.as_str()
            );
            return Err(StoreError::Invalid(
                "This paper already has an open appraisal; set it aside to start another.".into(),
            ));
        }
        let shown = fetch_one(
            tx,
            "SELECT 1 AS one FROM evidence_runs
             WHERE paper_id = ? AND first_shown_at IS NOT NULL LIMIT 1",
            &[Value::from(paper.as_str())],
        )?
        .is_some();
        execute(
            tx,
            "INSERT INTO evidence_appraisals (paper_id, canonical_id, blind) VALUES (?, ?, ?)",
            &[
                Value::from(paper.as_str()),
                Value::from(canonical_id),
                Value::from(!shown),
            ],
        )?;
        let id = last_id(tx)?;
        insert_version(tx, id, 0)?;
        log::info!(
            "appraisal {id} started on paper {} ({})",
            paper.as_str(),
            if shown { "not blind" } else { "blind" }
        );
        Ok(id)
    })
}

/// Confirms appraisal `appraisal_id`'s design and outcomes in one
/// transaction: stores them with `confirmed_at`, then readies its latest
/// paired run (`plan_paired`): RoB 2's plan for the confirmed outcomes, or
/// *not covered* and complete. Refused as [`StoreError::Invalid`], with
/// nothing written: a set-aside or already confirmed appraisal, no paired
/// run with its outcomes step done, a design or layout bmlib does not know,
/// a covered design with no outcome, a blank name, a result
/// `ReportedResult::validate` refuses, or `from_proposed` out of range or
/// repeated. No such appraisal is [`StoreError::NotFound`].
pub fn confirm(db: &mut dyn Db, appraisal_id: i64, input: &ConfirmInput) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let a = existing(tx, appraisal_id)?;
        if a.set_aside_at.is_some() {
            return Err(invalid(appraisal_id, "This appraisal was set aside."));
        }
        if a.confirmed_at.is_some() {
            return Err(invalid(
                appraisal_id,
                "Its design and outcomes are confirmed already; set it aside to start again.",
            ));
        }
        let covered = design_covered(&input.design, input.rct_layout.as_deref())
            .map_err(|why| invalid(appraisal_id, why))?;
        if covered && input.outcomes.is_empty() {
            return Err(invalid(
                appraisal_id,
                "At least one primary outcome is needed.",
            ));
        }
        let (run_id, proposed) = proposals(tx, &a)?;
        check_outcomes(&input.outcomes, proposed).map_err(|why| invalid(appraisal_id, why))?;
        execute(
            tx,
            "UPDATE evidence_appraisals SET design = ?, rct_layout = ?, confirmed_at = datetime('now')
             WHERE id = ?",
            &[
                Value::from(input.design.as_str()),
                Value::from(input.rct_layout.as_deref()),
                Value::from(appraisal_id),
            ],
        )?;
        for (position, o) in input.outcomes.iter().enumerate() {
            insert_outcome(tx, appraisal_id, position, o)?;
        }
        plan_paired(tx, run_id, covered, input.outcomes.len())?;
        log::info!(
            "appraisal {appraisal_id} confirmed with {} outcome(s); run {run_id} readied",
            input.outcomes.len()
        );
        Ok(())
    })
}

/// Writes `edit` to appraisal `appraisal_id`'s editable version in one
/// transaction, then re-derives that version. The editable version is the
/// first record until the save; after it, the open revision, made by
/// copying the latest version's rows when none is open. Refused as
/// [`StoreError::Invalid`], with nothing written: an appraisal not
/// confirmed, set aside or not covered, or an edit the rules refuse (D1
/// with an outcome or D2–D5 without a confirmed one, a question not of its
/// domain, an answer `allowed_answers` refuses, a mark outside the text, a
/// note over `APPRAISAL_NOTE_MAX_CHARS`). A write that would reach a frozen
/// or closed version is [`StoreError::Corrupt`].
pub fn edit(db: &mut dyn Db, appraisal_id: i64, edit: &AppraisalEdit) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let a = answering(tx, appraisal_id)?;
        check_edit(edit, a.outcomes.len(), &a.text).map_err(|why| invalid(appraisal_id, why))?;
        let (version_id, number) = match open_version(&a) {
            Some(v) => (v.id, v.number),
            None => new_revision(tx, &a)?,
        };
        check_writable(tx, version_id)?;
        apply(tx, version_id, edit)?;
        rederive(tx, appraisal_id, version_id)?;
        log::info!("appraisal {appraisal_id}: an edit written to version {number}");
        Ok(())
    })
}

/// Saves appraisal `appraisal_id`'s first record in one transaction: it is
/// frozen, the appraisal saved, and revealed if it was not already. Refused
/// as [`StoreError::Invalid`]: an appraisal not confirmed, set aside, not
/// covered or saved already, or a first record not complete.
pub fn save(db: &mut dyn Db, appraisal_id: i64) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let a = answering(tx, appraisal_id)?;
        if a.saved_at.is_some() {
            return Err(invalid(appraisal_id, "This appraisal is saved already."));
        }
        let first = a
            .versions
            .first()
            .ok_or_else(|| no_first_record(appraisal_id))?;
        check_writable(tx, first.id)?;
        if !complete(&rater_record(first), a.outcomes.len()) {
            return Err(invalid(
                appraisal_id,
                "Answer every question that applies, and each escalation asked, before saving.",
            ));
        }
        let now = now(tx)?;
        execute(
            tx,
            "UPDATE evidence_appraisal_versions SET frozen_at = ? WHERE id = ?",
            &[Value::from(now.as_str()), Value::from(first.id)],
        )?;
        execute(
            tx,
            "UPDATE evidence_appraisals SET saved_at = ?, revealed_at = IFNULL(revealed_at, ?)
             WHERE id = ?",
            &[
                Value::from(now.as_str()),
                Value::from(now.as_str()),
                Value::from(appraisal_id),
            ],
        )?;
        log::info!("appraisal {appraisal_id}: first record saved");
        Ok(())
    })
}

/// *Show the machine's verdict now*: before a save, marks appraisal
/// `appraisal_id` revealed (keeping an earlier reveal's time) and not blind,
/// freezing nothing. A saved or set-aside appraisal is [`StoreError::Invalid`].
pub fn reveal_now(db: &mut dyn Db, appraisal_id: i64) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let a = existing(tx, appraisal_id)?;
        if a.set_aside_at.is_some() {
            return Err(invalid(appraisal_id, "This appraisal was set aside."));
        }
        if a.saved_at.is_some() {
            return Err(invalid(
                appraisal_id,
                "This appraisal is saved, so the machine's verdict is shown already.",
            ));
        }
        execute(
            tx,
            "UPDATE evidence_appraisals SET revealed_at = IFNULL(revealed_at, datetime('now')),
                                            blind = 0
             WHERE id = ?",
            &[Value::from(appraisal_id)],
        )?;
        log::info!("appraisal {appraisal_id}: revealed before its save, not blind");
        Ok(())
    })
}

/// *Done revising*: closes appraisal `appraisal_id`'s open revision, so the
/// next edit opens another. None open, or a set-aside appraisal, is
/// [`StoreError::Invalid`].
pub fn close_revision(db: &mut dyn Db, appraisal_id: i64) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let a = existing(tx, appraisal_id)?;
        if a.set_aside_at.is_some() {
            return Err(invalid(appraisal_id, "This appraisal was set aside."));
        }
        let Some(v) = open_version(&a).filter(|v| v.number > 0) else {
            return Err(invalid(appraisal_id, "No revision is open."));
        };
        check_writable(tx, v.id)?;
        execute(
            tx,
            "UPDATE evidence_appraisal_versions SET closed_at = datetime('now') WHERE id = ?",
            &[Value::from(v.id)],
        )?;
        log::info!("appraisal {appraisal_id}: revision {} closed", v.number);
        Ok(())
    })
}

/// Sets appraisal `appraisal_id` aside: kept with every version, no longer
/// open, so a new one can start. It reads only the row, so a damaged
/// appraisal can be set aside too. Set aside already is
/// [`StoreError::Invalid`]; no such appraisal is [`StoreError::NotFound`].
pub fn set_aside(db: &mut dyn Db, appraisal_id: i64) -> Result<(), StoreError> {
    transaction_with(db, |tx| {
        let r = fetch_one(
            tx,
            "SELECT set_aside_at FROM evidence_appraisals WHERE id = ?",
            &[Value::from(appraisal_id)],
        )?
        .ok_or_else(|| StoreError::NotFound(format!("appraisal {appraisal_id}")))?;
        if !r.get("set_aside_at")?.is_null() {
            return Err(invalid(
                appraisal_id,
                "This appraisal was set aside already.",
            ));
        }
        execute(
            tx,
            "UPDATE evidence_appraisals SET set_aside_at = datetime('now') WHERE id = ?",
            &[Value::from(appraisal_id)],
        )?;
        log::info!("appraisal {appraisal_id} set aside");
        Ok(())
    })
}

/// Re-derives appraisal `appraisal_id`'s editable version (an unsaved first
/// record, or an open revision) under the current rules when it was made
/// under earlier ones, taking their versions, and returns whether it did. A
/// frozen or closed version keeps its own; a set-aside or not-covered
/// appraisal is left as it is.
pub fn refresh_editable(db: &mut dyn Db, appraisal_id: i64) -> Result<bool, StoreError> {
    transaction_with(db, |tx| {
        let a = existing(tx, appraisal_id)?;
        if a.set_aside_at.is_some() || a.state == AppraisalState::NotCovered {
            return Ok(false);
        }
        let Some(v) = open_version(&a).filter(|v| v.earlier_rules) else {
            return Ok(false);
        };
        check_writable(tx, v.id)?;
        execute(
            tx,
            "UPDATE evidence_appraisal_versions SET algorithm_version = ?, ladder_version = ?
             WHERE id = ?",
            &[
                Value::from(ROB2_ALGORITHM_VERSION),
                Value::from(LADDER_VERSION),
                Value::from(v.id),
            ],
        )?;
        rederive(tx, appraisal_id, v.id)?;
        log::info!(
            "appraisal {appraisal_id}: version {} re-derived under the current rules",
            v.number
        );
        Ok(true)
    })
}

/// The id of `paper`'s open (not set-aside) appraisal, if it has one.
pub fn open_appraisal(db: &mut dyn Db, paper: &PaperId) -> Result<Option<i64>, StoreError> {
    Ok(fetch_one(
        db,
        "SELECT id FROM evidence_appraisals WHERE paper_id = ? AND set_aside_at IS NULL",
        &[Value::from(paper.as_str())],
    )?
    .map(|r| r.get_i64("id"))
    .transpose()?)
}

/// The reader's record in version `v`, as completeness and conformance read
/// it: each domain's and outcome's verdict as left ([`effective`] of the
/// stored derived verdict and override), and only the answers that count. A
/// row with no answer (a mark or a tick alone) is unanswered. An answer to a
/// question the pinned bmlib does not have, or one it does not allow (as a
/// re-pin can leave), stays stored but is not counted, with a warning.
pub fn rater_record(v: &StoredVersion) -> RaterRecord {
    let mut r = RaterRecord {
        domains: v
            .domains
            .iter()
            .map(|d| RaterDomain {
                domain: d.domain,
                outcome: d.outcome,
                answers: Answers::new(),
                verdict: effective(d.derived, d.override_),
            })
            .collect(),
        overalls: v
            .overalls
            .iter()
            .map(|o| RaterOverall {
                outcome: o.outcome,
                escalate: o.escalate,
                overall: effective(o.derived, o.override_),
            })
            .collect(),
    };
    for x in &v.answers {
        let Some(answer) = x.answer else {
            continue;
        };
        let q = x.question_id.as_str();
        // `allowed_answers` panics on an unknown id, so the known check goes first.
        if !questions(x.domain).contains(&q) || !allowed_answers(q).contains(&answer) {
            log::warn!(
                "appraisal version {}: the answer {answer:?} to {q:?} of {} is kept but not \
                 counted: the current RoB 2 rules have no such question or answer",
                v.id,
                domain_name(x.domain)
            );
            continue;
        }
        let at = r
            .domains
            .iter()
            .position(|d| d.domain == x.domain && d.outcome == x.outcome);
        let at = at.unwrap_or_else(|| {
            r.domains.push(RaterDomain {
                domain: x.domain,
                outcome: x.outcome,
                answers: Answers::new(),
                verdict: None,
            });
            r.domains.len() - 1
        });
        r.domains[at].answers.insert(q.to_string(), answer);
    }
    r.domains.sort_by_key(|d| (d.domain, d.outcome));
    r
}

/// Re-derives version `version_id` of appraisal `appraisal_id` from its rows
/// and stores the result, inside the caller's transaction: every domain's
/// derived verdict (D1 and each outcome's D2–D5), then each outcome's overall
/// verdict and ladder from the domain verdicts as left. Every write to a
/// version calls it, so what is stored is what the current rules give.
fn rederive(tx: &mut dyn Db, appraisal_id: i64, version_id: i64) -> Result<(), StoreError> {
    let a = existing(tx, appraisal_id)?;
    let v = a
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| StoreError::NotFound(format!("appraisal version {version_id}")))?;
    let defect = |why: String| {
        StoreError::Corrupt(format!(
            "appraisal {appraisal_id}, version {}: {why}",
            v.number
        ))
    };
    let design = a.design.as_deref().unwrap_or_default();
    let start = start_level(study_design_from_str(design))
        .ok_or_else(|| defect(format!("design {design:?} has no ladder")))?;
    let counted = rater_record(v);
    let scopes = std::iter::once((Domain::D1, None))
        .chain((0..a.outcomes.len()).flat_map(|o| OUTCOME_DOMAINS.map(|d| (d, Some(o)))));
    let mut left = RaterRecord::default();
    for (domain, outcome) in scopes {
        let answers = counted
            .domain(domain, outcome)
            .map(|d| d.answers.clone())
            .unwrap_or_default();
        // `rater_record` counts only known questions and allowed answers.
        let d = derived(domain, &answers).map_err(|e| defect(e.to_string()))?;
        write_row(
            tx,
            version_id,
            RowWrite {
                table: DOMAINS,
                key: domain_key(domain, outcome),
                assignments: "derived = ?",
                values: vec![Value::from(d.as_ref().map(wire).transpose()?)],
                clears: d.is_none(),
                empty: Some(DOMAIN_EMPTY),
            },
        )?;
        let override_ = v
            .domains
            .iter()
            .find(|x| x.domain == domain && x.outcome == outcome)
            .and_then(|x| x.override_);
        left.domains.push(RaterDomain {
            domain,
            outcome,
            answers,
            verdict: effective(d, override_),
        });
    }
    for (o, confirmed) in a.outcomes.iter().enumerate() {
        let row = v.overalls.iter().find(|x| x.outcome == o);
        let verdict = outcome_verdict(
            left.outcome_domains(o),
            row.and_then(|r| r.escalate),
            row.and_then(|r| r.override_),
            start,
            confirmed.result.as_ref(),
        );
        if let Some(why) = &verdict.ladder_error {
            log::warn!("appraisal {appraisal_id}, outcome {o}: no ladder: {why}");
        }
        let ladder = verdict.ladder.as_ref();
        write_row(
            tx,
            version_id,
            RowWrite {
                table: OVERALLS,
                key: vec![("outcome", Value::from(o))],
                assignments: "derived = ?, ladder_start = ?, ladder_level = ?, ladder_weight = ?, \
                    ladder_error = ?, ladder_steps_json = ?",
                values: vec![
                    Value::from(verdict.derived_overall.as_ref().map(wire).transpose()?),
                    Value::from(ladder.map(|l| wire(&l.start)).transpose()?),
                    Value::from(ladder.map(|l| wire(&l.level)).transpose()?),
                    Value::from(ladder.map(|l| l.weight)),
                    Value::from(verdict.ladder_error.as_deref()),
                    Value::from(ladder.map(ladder_steps_json).transpose()?),
                ],
                clears: verdict.derived_overall.is_none()
                    && ladder.is_none()
                    && verdict.ladder_error.is_none(),
                empty: None,
            },
        )?;
    }
    Ok(())
}

/// Ladder `l`'s steps as JSON shaped as `LadderView.steps`.
fn ladder_steps_json(l: &EvidenceLadder) -> Result<String, StoreError> {
    let steps = l
        .steps
        .iter()
        .map(ladder_step_view)
        .collect::<Result<Vec<_>, StoreError>>()?;
    serde_json::to_string(&steps)
        .map_err(|e| StoreError::Corrupt(format!("a ladder's steps do not serialise: {e}")))
}

/// Refuses, as [`StoreError::Corrupt`], a write to version `version_id` when
/// it is frozen or closed: such a version is never written.
fn check_writable(db: &mut dyn Db, version_id: i64) -> Result<(), StoreError> {
    let r = fetch_one(
        db,
        "SELECT frozen_at, closed_at FROM evidence_appraisal_versions WHERE id = ?",
        &[Value::from(version_id)],
    )?
    .ok_or_else(|| StoreError::NotFound(format!("appraisal version {version_id}")))?;
    if r.get("frozen_at")?.is_null() && r.get("closed_at")?.is_null() {
        return Ok(());
    }
    Err(StoreError::Corrupt(format!(
        "appraisal version {version_id} is frozen or closed, and is never written"
    )))
}

/// A refusal of a request on appraisal `appraisal_id`, saying `why` to the
/// user; the id goes to the log only.
fn invalid(appraisal_id: i64, why: impl std::fmt::Display) -> StoreError {
    log::info!("appraisal {appraisal_id}: refused: {why}");
    StoreError::Invalid(why.to_string())
}

/// The error of an appraisal stored without a first record.
fn no_first_record(appraisal_id: i64) -> StoreError {
    StoreError::Corrupt(format!("appraisal {appraisal_id}: it has no first record"))
}

/// SQLite's time now, as every `*_at` column stores it.
fn now(db: &mut dyn Db) -> Result<String, StoreError> {
    Ok(fetch_one(db, "SELECT datetime('now') AS now", &[])?
        .ok_or_else(|| StoreError::Corrupt("SQLite gave no time".into()))?
        .get_str("now")?
        .to_string())
}

/// Appraisal `appraisal_id`, read; no such appraisal is [`StoreError::NotFound`].
fn existing(db: &mut dyn Db, appraisal_id: i64) -> Result<StoredAppraisal, StoreError> {
    appraisal(db, appraisal_id)?
        .ok_or_else(|| StoreError::NotFound(format!("appraisal {appraisal_id}")))
}

/// Appraisal `appraisal_id`, read, when the reader may answer in it: not set
/// aside, confirmed, and on a covered design. Otherwise [`StoreError::Invalid`].
fn answering(db: &mut dyn Db, appraisal_id: i64) -> Result<StoredAppraisal, StoreError> {
    let a = existing(db, appraisal_id)?;
    if a.set_aside_at.is_some() {
        return Err(invalid(appraisal_id, "This appraisal was set aside."));
    }
    match a.state {
        AppraisalState::Confirming => Err(invalid(
            appraisal_id,
            "Confirm the design and outcomes first.",
        )),
        AppraisalState::NotCovered => Err(invalid(
            appraisal_id,
            format!(
                "No method for this design yet: {}",
                a.design.as_deref().unwrap_or_default()
            ),
        )),
        AppraisalState::Answering | AppraisalState::Saved => Ok(a),
    }
}

/// The version still open to writes: the last, when it is neither frozen nor
/// closed.
fn open_version(a: &StoredAppraisal) -> Option<&StoredVersion> {
    a.versions
        .last()
        .filter(|v| v.frozen_at.is_none() && v.closed_at.is_none())
}

/// Inserts version `number` of appraisal `appraisal_id` under the current
/// rules, and returns its id.
fn insert_version(db: &mut dyn Db, appraisal_id: i64, number: u32) -> Result<i64, StoreError> {
    execute(
        db,
        "INSERT INTO evidence_appraisal_versions (appraisal_id, number, algorithm_version,
                                                  ladder_version)
         VALUES (?, ?, ?, ?)",
        &[
            Value::from(appraisal_id),
            Value::from(i64::from(number)),
            Value::from(ROB2_ALGORITHM_VERSION),
            Value::from(LADDER_VERSION),
        ],
    )?;
    last_id(db)
}

/// Opens the next revision of saved appraisal `a`, a copy of its latest
/// version's rows under the current rules, and returns its id and number.
fn new_revision(db: &mut dyn Db, a: &StoredAppraisal) -> Result<(i64, u32), StoreError> {
    let latest = a.versions.last().ok_or_else(|| no_first_record(a.id))?;
    let number = latest.number + 1;
    let id = insert_version(db, a.id, number)?;
    for (table, columns) in [
        (ANSWERS, ANSWER_COLUMNS),
        (DOMAINS, DOMAIN_COLUMNS),
        (OVERALLS, OVERALL_COLUMNS),
    ] {
        execute(
            db,
            &format!(
                "INSERT INTO {table} (version_id, {columns})
                 SELECT ?, {columns} FROM {table} WHERE version_id = ?"
            ),
            &[Value::from(id), Value::from(latest.id)],
        )?;
    }
    log::info!(
        "appraisal {}: revision {} opened from version {}",
        a.id,
        number,
        latest.number
    );
    Ok((id, number))
}

/// Appraisal `a`'s latest paired run on its text, with its outcomes step
/// done, and how many outcomes the machine proposed. None such is
/// [`StoreError::Invalid`]: confirming needs both proposals stored.
fn proposals(db: &mut dyn Db, a: &StoredAppraisal) -> Result<(i64, usize), StoreError> {
    let row = fetch_one(
        db,
        "SELECT r.id AS run_id, s.status,
                (SELECT count(*) FROM evidence_outcomes o WHERE o.step_id = s.id) AS n
         FROM evidence_runs r LEFT JOIN evidence_steps s ON s.run_id = r.id AND s.kind = ?
         WHERE r.appraisal_id = ? AND r.canonical_id = ? ORDER BY r.id DESC LIMIT 1",
        &[
            Value::from(STEP_OUTCOMES),
            Value::from(a.id),
            Value::from(a.canonical_id),
        ],
    )?;
    match row {
        Some(r) if r.get("status")?.as_str() == Some(StepStatus::Done.as_str()) => {
            let n = r.get_i64("n")?;
            let n = usize::try_from(n)
                .map_err(|_| StoreError::Corrupt(format!("a count of {n} outcomes")))?;
            Ok((r.get_i64("run_id")?, n))
        }
        _ => Err(invalid(
            a.id,
            "The machine's design and outcome proposals are not stored yet; continue its run first.",
        )),
    }
}

/// Refuses confirmed outcomes with a blank name, a result
/// `ReportedResult::validate` refuses, or a `from_proposed` that is not one
/// of the `proposed` outcomes or names one twice.
fn check_outcomes(outcomes: &[ConfirmedOutcomeInput], proposed: usize) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for (i, o) in outcomes.iter().enumerate() {
        if o.name.trim().is_empty() {
            return Err(format!("outcome {i} has no name"));
        }
        if let Some(from) = o.from_proposed {
            if from >= proposed {
                return Err(format!(
                    "outcome {i} comes from proposal {from}, but {proposed} were proposed"
                ));
            }
            if !seen.insert(from) {
                return Err(format!("proposal {from} is confirmed twice"));
            }
        }
        if let Some(r) = &o.result {
            r.validate()
                .map_err(|e| format!("outcome {i}'s result: {e}"))?;
        }
    }
    Ok(())
}

/// Stores confirmed outcome `o` of appraisal `appraisal_id` at `position`.
fn insert_outcome(
    db: &mut dyn Db,
    appraisal_id: i64,
    position: usize,
    o: &ConfirmedOutcomeInput,
) -> Result<(), StoreError> {
    let r = o.result.as_ref();
    execute(
        db,
        "INSERT INTO evidence_appraisal_outcomes (appraisal_id, position, from_proposed, name,
                                                  timepoint, measure, scale, estimate, ci_low,
                                                  ci_high, ci_level, n_analysed)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::from(appraisal_id),
            Value::from(position),
            Value::from(o.from_proposed),
            Value::from(o.name.as_str()),
            Value::from(not_blank(o.timepoint.as_deref())),
            Value::from(r.map(|r| r.measure.as_str())),
            Value::from(r.map(|r| wire(&r.scale)).transpose()?),
            Value::from(r.map(|r| r.estimate)),
            Value::from(r.and_then(|r| r.ci_low)),
            Value::from(r.and_then(|r| r.ci_high)),
            Value::from(r.and_then(|r| r.ci_level)),
            Value::from(r.and_then(|r| r.n_analysed).map(i64::from)),
        ],
    )?;
    Ok(())
}

/// Refuses `edit` unless every field holds on an appraisal with `outcomes`
/// confirmed outcomes on `text`: a domain row's scope ([`check_scope`]), a
/// question of its domain, an answer in `allowed_answers` (so no NI on 3.2),
/// a mark inside the text, an outcome confirmed, and a note of at most
/// [`APPRAISAL_NOTE_MAX_CHARS`] characters.
fn check_edit(edit: &AppraisalEdit, outcomes: usize, text: &CanonicalText) -> Result<(), String> {
    match edit {
        AppraisalEdit::Answer {
            domain,
            outcome,
            question_id,
            answer,
        } => {
            check_question(*domain, *outcome, question_id, outcomes)?;
            if let Some(a) = answer.filter(|a| !allowed_answers(question_id).contains(a)) {
                return Err(format!("RoB 2 does not allow {a:?} on {question_id}"));
            }
        }
        AppraisalEdit::Mark {
            domain,
            outcome,
            question_id,
            span,
        } => {
            check_question(*domain, *outcome, question_id, outcomes)?;
            if let Some(span) = span {
                check_span(text, *span)?;
            }
        }
        AppraisalEdit::Sources {
            domain,
            outcome,
            question_id,
            ..
        } => check_question(*domain, *outcome, question_id, outcomes)?,
        AppraisalEdit::Override {
            domain, outcome, ..
        } => check_scope(*domain, *outcome, outcomes)?,
        AppraisalEdit::Note {
            domain,
            outcome,
            note,
        } => {
            check_scope(*domain, *outcome, outcomes)?;
            let chars = note.as_deref().map_or(0, |n| n.chars().count());
            if chars > APPRAISAL_NOTE_MAX_CHARS {
                return Err(format!(
                    "the note has {chars} characters; at most {APPRAISAL_NOTE_MAX_CHARS} are kept"
                ));
            }
        }
        AppraisalEdit::Escalate { outcome, .. }
        | AppraisalEdit::OverallOverride { outcome, .. } => check_outcome(*outcome, outcomes)?,
    }
    Ok(())
}

/// The condition under which an answer row holds nothing: no answer, mark
/// or tick.
const ANSWER_EMPTY: &str = "answer IS NULL AND mark_start IS NULL AND mark_end IS NULL \
    AND used_protocol = 0 AND used_registry = 0 AND used_supplement = 0";
/// The condition under which a domain row holds nothing: no derived verdict,
/// override or note.
const DOMAIN_EMPTY: &str = "derived IS NULL AND override IS NULL AND note IS NULL";

/// `text`, or `None` when it is empty or only whitespace: the reader cleared
/// it. Text with anything in it is kept whole, never trimmed.
fn not_blank(text: Option<&str>) -> Option<&str> {
    text.filter(|t| !t.trim().is_empty())
}

/// One write to a row of a version, as [`apply`] builds it.
struct RowWrite {
    /// The row's table.
    table: &'static str,
    /// The row's key: column and value pairs, NULL matching NULL.
    key: Vec<(&'static str, Value)>,
    /// What is set (`"a = ?, b = ?"`).
    assignments: &'static str,
    /// The values set, in order.
    values: Vec<Value>,
    /// Whether the write clears what it sets: then a missing row is not made.
    clears: bool,
    /// The SQL condition under which the row holds nothing and is deleted,
    /// if such a row goes.
    empty: Option<&'static str>,
}

/// Makes write `w` on version `version_id`'s row. A missing row is inserted
/// first, unless the write clears: then nothing is written. A row the write
/// leaves holding nothing (`w.empty`) is deleted.
fn write_row(db: &mut dyn Db, version_id: i64, w: RowWrite) -> Result<(), StoreError> {
    let RowWrite {
        table,
        key,
        assignments,
        mut values,
        clears,
        empty,
    } = w;
    let mut keyed = vec![Value::from(version_id)];
    keyed.extend(key.iter().map(|(_, v)| v.clone()));
    let matches: String = key.iter().map(|(c, _)| format!(" AND {c} IS ?")).collect();
    let found = fetch_one(
        db,
        &format!("SELECT id FROM {table} WHERE version_id = ?{matches}"),
        &keyed,
    )?;
    let id = match found {
        Some(r) => r.get_i64("id")?,
        None if clears => return Ok(()),
        None => {
            let columns: Vec<&str> = key.iter().map(|(c, _)| *c).collect();
            let marks = vec!["?"; keyed.len()].join(", ");
            execute(
                db,
                &format!(
                    "INSERT INTO {table} (version_id, {}) VALUES ({marks})",
                    columns.join(", ")
                ),
                &keyed,
            )?;
            last_id(db)?
        }
    };
    values.push(Value::from(id));
    execute(
        db,
        &format!("UPDATE {table} SET {assignments} WHERE id = ?"),
        &values,
    )?;
    if let Some(empty) = empty {
        execute(
            db,
            &format!("DELETE FROM {table} WHERE id = ? AND {empty}"),
            &[Value::from(id)],
        )?;
    }
    Ok(())
}

/// The key of a domain row: its domain and outcome.
fn domain_key(domain: Domain, outcome: Option<usize>) -> Vec<(&'static str, Value)> {
    vec![
        ("domain", Value::from(domain_name(domain))),
        ("outcome", Value::from(outcome)),
    ]
}

/// The key of an answer row: its domain, outcome and question.
fn answer_key(
    domain: Domain,
    outcome: Option<usize>,
    question_id: &str,
) -> Vec<(&'static str, Value)> {
    let mut key = domain_key(domain, outcome);
    key.push(("question_id", Value::from(question_id)));
    key
}

/// Writes `edit`, already checked, to version `version_id`'s row it names.
/// An answer or domain row left holding nothing is deleted.
fn apply(db: &mut dyn Db, version_id: i64, edit: &AppraisalEdit) -> Result<(), StoreError> {
    let judgement = |j: &Option<Judgement>| -> Result<Value, StoreError> {
        Ok(Value::from(j.as_ref().map(wire).transpose()?))
    };
    let w = match edit {
        AppraisalEdit::Answer {
            domain,
            outcome,
            question_id,
            answer,
        } => RowWrite {
            table: ANSWERS,
            key: answer_key(*domain, *outcome, question_id),
            assignments: "answer = ?",
            values: vec![Value::from(answer.as_ref().map(wire).transpose()?)],
            clears: answer.is_none(),
            empty: Some(ANSWER_EMPTY),
        },
        AppraisalEdit::Mark {
            domain,
            outcome,
            question_id,
            span,
        } => RowWrite {
            table: ANSWERS,
            key: answer_key(*domain, *outcome, question_id),
            assignments: "mark_start = ?, mark_end = ?",
            values: vec![
                Value::from(span.map(|s| s.start)),
                Value::from(span.map(|s| s.end)),
            ],
            clears: span.is_none(),
            empty: Some(ANSWER_EMPTY),
        },
        AppraisalEdit::Sources {
            domain,
            outcome,
            question_id,
            protocol,
            registry,
            supplement,
        } => RowWrite {
            table: ANSWERS,
            key: answer_key(*domain, *outcome, question_id),
            assignments: "used_protocol = ?, used_registry = ?, used_supplement = ?",
            values: vec![
                Value::from(*protocol),
                Value::from(*registry),
                Value::from(*supplement),
            ],
            clears: !(*protocol || *registry || *supplement),
            empty: Some(ANSWER_EMPTY),
        },
        AppraisalEdit::Override {
            domain,
            outcome,
            judgement: j,
        } => RowWrite {
            table: DOMAINS,
            key: domain_key(*domain, *outcome),
            assignments: "override = ?",
            values: vec![judgement(j)?],
            clears: j.is_none(),
            empty: Some(DOMAIN_EMPTY),
        },
        AppraisalEdit::Note {
            domain,
            outcome,
            note,
        } => RowWrite {
            table: DOMAINS,
            key: domain_key(*domain, *outcome),
            assignments: "note = ?",
            values: vec![Value::from(not_blank(note.as_deref()))],
            clears: not_blank(note.as_deref()).is_none(),
            empty: Some(DOMAIN_EMPTY),
        },
        AppraisalEdit::Escalate { outcome, escalate } => RowWrite {
            table: OVERALLS,
            key: vec![("outcome", Value::from(*outcome))],
            assignments: "escalate = ?",
            values: vec![Value::from(*escalate)],
            clears: escalate.is_none(),
            empty: None,
        },
        AppraisalEdit::OverallOverride {
            outcome,
            judgement: j,
        } => RowWrite {
            table: OVERALLS,
            key: vec![("outcome", Value::from(*outcome))],
            assignments: "override = ?",
            values: vec![judgement(j)?],
            clears: j.is_none(),
            empty: None,
        },
    };
    write_row(db, version_id, w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appraisal::{departures, Departure, DepartureScope};
    use crate::appraisal_rows::{has_marks, ReaderOverall};
    use crate::config::APPRAISAL_NOTE_MAX_CHARS;
    use crate::evidence_store::tests::{kind_and_status, save_pending, save_step};
    use crate::evidence_store::{create_run, pending_steps, EvidenceStep};
    use crate::pico_store::tests::{count, paper_with_pmid, paper_with_text};
    use crate::store::open_memory_store;
    use bmlib::db::{execute, fetch_one, Value};
    use bmlib::quality::{plan_rob2, EffectScale};
    use rusqlite::Connection;

    /// The appraisal tables, children after parents.
    const TABLES: [&str; 6] = [
        "evidence_appraisals",
        "evidence_appraisal_outcomes",
        "evidence_appraisal_versions",
        "evidence_appraisal_answers",
        "evidence_appraisal_domains",
        "evidence_appraisal_overalls",
    ];

    /// A coherent result: a mean difference with its 95% interval.
    fn result() -> ReportedResult {
        ReportedResult {
            measure: "mean difference".into(),
            scale: EffectScale::Difference,
            estimate: -0.4,
            ci_low: Some(-0.7),
            ci_high: Some(-0.1),
            ci_level: Some(0.95),
            n_analysed: Some(312),
        }
    }

    /// A parallel-group trial with `n` outcomes, each from the proposal at
    /// its own index.
    fn input(n: usize) -> ConfirmInput {
        ConfirmInput {
            design: "rct".into(),
            rct_layout: Some("parallel".into()),
            outcomes: (0..n)
                .map(|i| ConfirmedOutcomeInput {
                    from_proposed: Some(i),
                    name: format!("Confirmed {i}"),
                    timepoint: Some("12 months".into()),
                    result: Some(result()),
                })
                .collect(),
        }
    }

    /// Pairs a new run on `cid` with appraisal `aid`, its design step pending.
    fn pair(c: &mut Connection, p: &PaperId, cid: i64, aid: i64) -> i64 {
        create_run(c, p, cid, "m", Some(aid)).unwrap()
    }

    /// As [`pair`], left waiting with its design and two outcomes proposed:
    /// a paired run stops there on its own.
    fn paired_run(c: &mut Connection, p: &PaperId, cid: i64, aid: i64) -> i64 {
        let run = pair(c, p, cid, aid);
        save_pending(c, p, cid, run, &[]);
        run
    }

    /// A paper, its text, a started appraisal and its waiting paired run.
    fn proposed(c: &mut Connection) -> (PaperId, i64, i64, i64) {
        let (p, cid) = paper_with_text(c);
        let aid = start_appraisal(c, &p).unwrap();
        let run = paired_run(c, &p, cid, aid);
        (p, cid, aid, run)
    }

    /// As [`proposed`], confirmed with two outcomes.
    fn confirmed(c: &mut Connection) -> (i64, i64, i64) {
        let (_, cid, aid, run) = proposed(c);
        confirm(c, aid, &input(2)).unwrap();
        (cid, aid, run)
    }

    /// The stored appraisal `aid`.
    fn stored(c: &mut Connection, aid: i64) -> StoredAppraisal {
        appraisal(c, aid).unwrap().unwrap()
    }

    /// An answer edit.
    fn answer(domain: Domain, outcome: Option<usize>, q: &str, a: Answer) -> AppraisalEdit {
        AppraisalEdit::Answer {
            domain,
            outcome,
            question_id: q.into(),
            answer: Some(a),
        }
    }

    /// A mark edit on D1's question `q`.
    fn mark(q: &str, span: Option<TextSpan>) -> AppraisalEdit {
        AppraisalEdit::Mark {
            domain: Domain::D1,
            outcome: None,
            question_id: q.into(),
            span,
        }
    }

    /// The question ids and answers of `v`, in order.
    fn answered(v: &StoredVersion) -> Vec<(String, Option<Answer>)> {
        v.answers
            .iter()
            .map(|a| (a.question_id.clone(), a.answer))
            .collect()
    }

    /// Every question of D1 and of each of `n` outcomes' D2–D5 that applies,
    /// answered so each domain is low.
    fn low_answers(n: usize) -> Vec<AppraisalEdit> {
        use Answer::{N, Y};
        let mut edits: Vec<AppraisalEdit> = [("1.1", Y), ("1.2", Y), ("1.3", N)]
            .into_iter()
            .map(|(q, a)| answer(Domain::D1, None, q, a))
            .collect();
        for o in 0..n {
            for (d, q, a) in [
                (Domain::D2, "2.1", N),
                (Domain::D2, "2.2", N),
                (Domain::D2, "2.6", Y),
                (Domain::D3, "3.1", Y),
                (Domain::D4, "4.1", N),
                (Domain::D4, "4.2", N),
                (Domain::D4, "4.3", N),
                (Domain::D5, "5.1", Y),
                (Domain::D5, "5.2", N),
                (Domain::D5, "5.3", N),
            ] {
                edits.push(answer(d, Some(o), q, a));
            }
        }
        edits
    }

    /// Answers appraisal `aid` low throughout for `n` outcomes.
    fn answer_low(c: &mut Connection, aid: i64, n: usize) {
        for e in low_answers(n) {
            edit(c, aid, &e).unwrap();
        }
    }

    /// The answer `v` stores to question `q` of `domain` for `outcome`.
    fn answer_in(
        v: &StoredVersion,
        domain: Domain,
        outcome: Option<usize>,
        q: &str,
    ) -> Option<Answer> {
        v.answers
            .iter()
            .find(|x| x.domain == domain && x.outcome == outcome && x.question_id == q)
            .and_then(|x| x.answer)
    }

    /// The derived verdict `v` stores for `domain` of `outcome`.
    fn derived_in(v: &StoredVersion, domain: Domain, outcome: Option<usize>) -> Option<Judgement> {
        v.domains
            .iter()
            .find(|d| d.domain == domain && d.outcome == outcome)
            .and_then(|d| d.derived)
    }

    /// `v`'s overall row for `outcome`.
    fn overall_in(v: &StoredVersion, outcome: usize) -> &ReaderOverall {
        v.overalls.iter().find(|o| o.outcome == outcome).unwrap()
    }

    /// Asserts `r` was refused as invalid input.
    fn refused<T: std::fmt::Debug>(r: Result<T, StoreError>) {
        assert!(matches!(r, Err(StoreError::Invalid(_))), "{r:?}");
    }

    /// Asserts `r` was refused as corrupt.
    fn corrupt<T: std::fmt::Debug>(r: Result<T, StoreError>) {
        assert!(matches!(r, Err(StoreError::Corrupt(_))), "{r:?}");
    }

    /// Runs `sql` on `c`.
    fn sql(c: &mut Connection, sql: &str) {
        execute(c, sql, &[]).unwrap();
    }

    /// A paper with no run shown starts blind, with an empty first record
    /// under the current rules; once a run of it was shown, the next starts
    /// not blind.
    #[test]
    fn start_is_blind_only_when_nothing_was_shown() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let a = start_appraisal(&mut c, &p).unwrap();
        let got = stored(&mut c, a);
        assert!(got.blind);
        assert_eq!(got.state, AppraisalState::Confirming);
        assert_eq!(got.canonical_id, cid);
        assert_eq!(got.versions.len(), 1);
        let v0 = &got.versions[0];
        assert_eq!(v0.number, 0);
        assert!(!v0.earlier_rules);
        assert!(v0.frozen_at.is_none() && v0.answers.is_empty());
        set_aside(&mut c, a).unwrap();
        let run = create_run(&mut c, &p, cid, "m", None).unwrap();
        execute(
            &mut c,
            "UPDATE evidence_runs SET first_shown_at = datetime('now') WHERE id = ?",
            &[Value::from(run)],
        )
        .unwrap();
        let b = start_appraisal(&mut c, &p).unwrap();
        assert!(!stored(&mut c, b).blind);
    }

    /// A second appraisal of a paper with one open is refused; another
    /// paper has its own.
    #[test]
    fn one_open_appraisal_per_paper() {
        let mut c = open_memory_store().unwrap();
        let (p, _) = paper_with_text(&mut c);
        let a = start_appraisal(&mut c, &p).unwrap();
        refused(start_appraisal(&mut c, &p));
        assert_eq!(count(&mut c, "evidence_appraisals"), 1);
        assert_eq!(open_appraisal(&mut c, &p).unwrap(), Some(a));
        let (q, _) = paper_with_pmid(&mut c, "2");
        let b = start_appraisal(&mut c, &q).unwrap();
        assert_ne!(a, b);
        assert_eq!(open_appraisal(&mut c, &q).unwrap(), Some(b));
    }

    /// A set-aside appraisal is kept, readable, and makes room for a new one;
    /// it cannot be set aside twice.
    #[test]
    fn set_aside_allows_a_new_one() {
        let mut c = open_memory_store().unwrap();
        let (p, _) = paper_with_text(&mut c);
        let a = start_appraisal(&mut c, &p).unwrap();
        set_aside(&mut c, a).unwrap();
        refused(set_aside(&mut c, a));
        assert_eq!(open_appraisal(&mut c, &p).unwrap(), None);
        assert!(stored(&mut c, a).set_aside_at.is_some());
        let b = start_appraisal(&mut c, &p).unwrap();
        assert_ne!(a, b);
        assert_eq!(open_appraisal(&mut c, &p).unwrap(), Some(b));
        assert_eq!(count(&mut c, "evidence_appraisals"), 2);
    }

    /// Confirming needs the paired run's outcomes step done; it then plans
    /// RoB 2 for the confirmed outcomes, and is final.
    #[test]
    fn confirm_needs_the_proposals() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let a = start_appraisal(&mut c, &p).unwrap();
        refused(confirm(&mut c, a, &input(2)));
        let run = pair(&mut c, &p, cid, a);
        refused(confirm(&mut c, a, &input(2)));
        let design = pending_steps(&mut c, run).unwrap().remove(0);
        save_step(&mut c, &p, cid, &design);
        refused(confirm(&mut c, a, &input(2)));
        let outcomes = pending_steps(&mut c, run).unwrap().remove(0);
        save_step(&mut c, &p, cid, &outcomes);
        assert!(pending_steps(&mut c, run).unwrap().is_empty());
        assert!(stored(&mut c, a).confirmed_at.is_none());
        confirm(&mut c, a, &input(2)).unwrap();
        let got = stored(&mut c, a);
        assert_eq!(got.state, AppraisalState::Answering);
        assert_eq!(got.design.as_deref(), Some("rct"));
        assert_eq!(got.rct_layout.as_deref(), Some("parallel"));
        let names: Vec<(usize, Option<usize>, &str)> = got
            .outcomes
            .iter()
            .map(|o| (o.position, o.from_proposed, o.name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![(0, Some(0), "Confirmed 0"), (1, Some(1), "Confirmed 1")]
        );
        assert_eq!(got.outcomes[0].result, Some(result()));
        let pending: Vec<EvidenceStep> = pending_steps(&mut c, run)
            .unwrap()
            .into_iter()
            .map(|s| s.step)
            .collect();
        let want: Vec<EvidenceStep> = plan_rob2(2).into_iter().map(EvidenceStep::Rob2).collect();
        assert_eq!(pending, want);
        refused(confirm(&mut c, a, &input(2)));
    }

    /// Every field the webview sends is checked before anything is written:
    /// an unknown design or layout, a layout missing or misplaced, no
    /// outcome, a bad interval, a blank name, `from_proposed` repeated or
    /// out of range. An outcome the reader added has no `from_proposed`.
    #[test]
    fn confirm_validates_its_input() {
        let mut c = open_memory_store().unwrap();
        let (_, _, a, run) = proposed(&mut c);
        let mut bad = Vec::new();
        let mut i = input(2);
        i.design = "trial".into();
        bad.push(i);
        let mut i = input(2);
        i.rct_layout = None;
        bad.push(i);
        let mut i = input(2);
        i.rct_layout = Some("sideways".into());
        bad.push(i);
        let mut i = input(0);
        i.design = "cohort_prospective".into();
        bad.push(i);
        bad.push(input(0));
        let mut i = input(1);
        i.outcomes[0].result.as_mut().unwrap().ci_low = Some(0.5);
        bad.push(i);
        let mut i = input(1);
        i.outcomes[0].name = "  ".into();
        bad.push(i);
        let mut i = input(2);
        i.outcomes[1].from_proposed = Some(0);
        bad.push(i);
        let mut i = input(1);
        i.outcomes[0].from_proposed = Some(2);
        bad.push(i);
        for i in &bad {
            refused(confirm(&mut c, a, i));
        }
        assert!(stored(&mut c, a).confirmed_at.is_none());
        assert_eq!(count(&mut c, "evidence_appraisal_outcomes"), 0);
        assert!(pending_steps(&mut c, run).unwrap().is_empty());
        let mut i = input(2);
        i.outcomes[1].from_proposed = None;
        confirm(&mut c, a, &i).unwrap();
        assert_eq!(stored(&mut c, a).outcomes[1].from_proposed, None);
    }

    /// A design not covered is confirmed with no outcome, ends the paired
    /// run as not covered and complete, and takes no edit or save.
    #[test]
    fn a_design_not_covered_stops_the_appraisal() {
        let mut c = open_memory_store().unwrap();
        let (_, _, a, run) = proposed(&mut c);
        let mut i = input(0);
        i.rct_layout = Some("crossover".into());
        confirm(&mut c, a, &i).unwrap();
        assert_eq!(stored(&mut c, a).state, AppraisalState::NotCovered);
        assert_eq!(
            kind_and_status(&mut c, run),
            (Some("not_covered".into()), "complete".into())
        );
        refused(edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)));
        refused(save(&mut c, a));
    }

    /// Before confirming, and on a set-aside appraisal, no edit, save,
    /// reveal or close is taken.
    #[test]
    fn writes_are_refused_before_confirming_and_once_set_aside() {
        let mut c = open_memory_store().unwrap();
        let (_, _, a, _) = proposed(&mut c);
        let y = answer(Domain::D1, None, "1.1", Answer::Y);
        refused(edit(&mut c, a, &y));
        refused(save(&mut c, a));
        confirm(&mut c, a, &input(1)).unwrap();
        set_aside(&mut c, a).unwrap();
        refused(edit(&mut c, a, &y));
        refused(save(&mut c, a));
        refused(reveal_now(&mut c, a));
        refused(close_revision(&mut c, a));
        assert!(!refresh_editable(&mut c, a).unwrap());
        assert_eq!(count(&mut c, "evidence_appraisal_answers"), 0);
    }

    /// Until the save every write lands in the first record: answers,
    /// marks, ticks, overrides, escalations and notes.
    #[test]
    fn edits_go_to_version_0_until_saved() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        let span = TextSpan { start: 0, end: 4 };
        for e in [
            answer(Domain::D1, None, "1.1", Answer::Y),
            answer(Domain::D2, Some(1), "2.1", Answer::Pn),
            mark("1.1", Some(span)),
            AppraisalEdit::Sources {
                domain: Domain::D1,
                outcome: None,
                question_id: "1.2".into(),
                protocol: true,
                registry: false,
                supplement: true,
            },
            AppraisalEdit::Override {
                domain: Domain::D2,
                outcome: Some(0),
                judgement: Some(Judgement::High),
            },
            AppraisalEdit::Note {
                domain: Domain::D2,
                outcome: Some(0),
                note: Some("Open label.".into()),
            },
            AppraisalEdit::Escalate {
                outcome: 0,
                escalate: Some(true),
            },
            AppraisalEdit::OverallOverride {
                outcome: 1,
                judgement: Some(Judgement::Low),
            },
        ] {
            edit(&mut c, a, &e).unwrap();
        }
        let got = stored(&mut c, a);
        assert_eq!(got.versions.len(), 1);
        let v0 = &got.versions[0];
        let one = v0.answers.iter().find(|x| x.question_id == "1.1").unwrap();
        assert_eq!((one.answer, one.mark), (Some(Answer::Y), Some(span)));
        let two = v0.answers.iter().find(|x| x.question_id == "1.2").unwrap();
        assert_eq!(two.answer, None);
        assert!(two.protocol && !two.registry && two.supplement);
        let d2 = v0.answers.iter().find(|x| x.question_id == "2.1").unwrap();
        assert_eq!((d2.outcome, d2.answer), (Some(1), Some(Answer::Pn)));
        assert_eq!(v0.domains.len(), 1);
        let dom = &v0.domains[0];
        assert_eq!(
            (dom.domain, dom.outcome, dom.override_, dom.note.as_deref()),
            (
                Domain::D2,
                Some(0),
                Some(Judgement::High),
                Some("Open label.")
            )
        );
        let overalls: Vec<(usize, Option<bool>, Option<Judgement>)> = v0
            .overalls
            .iter()
            .map(|o| (o.outcome, o.escalate, o.override_))
            .collect();
        assert_eq!(
            overalls,
            vec![(0, Some(true), None), (1, None, Some(Judgement::Low))]
        );
        let clear = AppraisalEdit::Answer {
            domain: Domain::D1,
            outcome: None,
            question_id: "1.1".into(),
            answer: None,
        };
        edit(&mut c, a, &clear).unwrap();
        let v0 = stored(&mut c, a).versions.remove(0);
        let one = v0.answers.iter().find(|x| x.question_id == "1.1").unwrap();
        assert_eq!((one.answer, one.mark), (None, Some(span)));
    }

    /// The webview is a trust boundary: an edit off its domain's scope, an
    /// outcome out of range, a question of another domain, NI on 3.2, a mark
    /// outside the text or empty, or a note over the limit is refused, and
    /// nothing is written. A note of exactly the limit, counted in
    /// characters, is taken.
    #[test]
    fn an_edit_is_validated() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        let long = "é".repeat(APPRAISAL_NOTE_MAX_CHARS + 1);
        for e in [
            answer(Domain::D1, Some(0), "1.1", Answer::Y),
            answer(Domain::D2, None, "2.1", Answer::Y),
            answer(Domain::D2, Some(2), "2.1", Answer::Y),
            answer(Domain::D1, None, "2.1", Answer::Y),
            answer(Domain::D3, Some(0), "3.2", Answer::Ni),
            answer(Domain::D3, Some(0), "3.9", Answer::Y),
            mark(
                "1.1",
                Some(TextSpan {
                    start: 0,
                    end: 100_000,
                }),
            ),
            mark("1.1", Some(TextSpan { start: 3, end: 3 })),
            AppraisalEdit::Override {
                domain: Domain::D1,
                outcome: Some(0),
                judgement: Some(Judgement::Low),
            },
            AppraisalEdit::Escalate {
                outcome: 2,
                escalate: Some(false),
            },
            AppraisalEdit::OverallOverride {
                outcome: 5,
                judgement: None,
            },
            AppraisalEdit::Note {
                domain: Domain::D1,
                outcome: None,
                note: Some(long),
            },
        ] {
            refused(edit(&mut c, a, &e));
        }
        for table in &TABLES[3..] {
            assert_eq!(count(&mut c, table), 0, "{table}");
        }
        let full = AppraisalEdit::Note {
            domain: Domain::D1,
            outcome: None,
            note: Some("é".repeat(APPRAISAL_NOTE_MAX_CHARS)),
        };
        edit(&mut c, a, &full).unwrap();
    }

    /// A write that leaves an answer or domain row holding nothing deletes
    /// it, and one that clears a missing row writes none; a blank note or
    /// time point is stored as none, while other text is kept untrimmed.
    #[test]
    fn a_cleared_row_is_deleted_and_blank_text_is_none() {
        let mut c = open_memory_store().unwrap();
        let (_, cid, a, _) = proposed(&mut c);
        let mut i = input(2);
        i.outcomes[0].timepoint = Some("  ".into());
        i.outcomes[1].timepoint = Some(" 12 months ".into());
        confirm(&mut c, a, &i).unwrap();
        let times: Vec<Option<String>> = stored(&mut c, a)
            .outcomes
            .into_iter()
            .map(|o| o.timepoint)
            .collect();
        assert_eq!(times, vec![None, Some(" 12 months ".into())]);
        let note = |text: &str| AppraisalEdit::Note {
            domain: Domain::D1,
            outcome: None,
            note: Some(text.into()),
        };
        let clear = AppraisalEdit::Answer {
            domain: Domain::D1,
            outcome: None,
            question_id: "1.1".into(),
            answer: None,
        };
        let no_ticks = AppraisalEdit::Sources {
            domain: Domain::D1,
            outcome: None,
            question_id: "1.2".into(),
            protocol: false,
            registry: false,
            supplement: false,
        };
        let no_override = AppraisalEdit::Override {
            domain: Domain::D1,
            outcome: None,
            judgement: None,
        };
        let no_escalate = AppraisalEdit::Escalate {
            outcome: 0,
            escalate: None,
        };
        for e in [&clear, &no_ticks, &no_override, &note(" \n "), &no_escalate] {
            edit(&mut c, a, e).unwrap();
        }
        for table in &TABLES[3..] {
            assert_eq!(count(&mut c, table), 0, "{table}");
        }
        edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
        edit(&mut c, a, &mark("1.1", Some(TextSpan { start: 0, end: 4 }))).unwrap();
        edit(&mut c, a, &clear).unwrap();
        assert_eq!(count(&mut c, "evidence_appraisal_answers"), 1);
        edit(&mut c, a, &mark("1.1", None)).unwrap();
        assert_eq!(count(&mut c, "evidence_appraisal_answers"), 0);
        assert!(!has_marks(&mut c, cid).unwrap());
        edit(&mut c, a, &note(" kept ")).unwrap();
        let v0 = stored(&mut c, a).versions.remove(0);
        assert_eq!(v0.domains[0].note.as_deref(), Some(" kept "));
        edit(&mut c, a, &note("")).unwrap();
        assert_eq!(count(&mut c, "evidence_appraisal_domains"), 0);
    }

    /// Saving freezes the first record and reveals; the next edit opens
    /// *after reveal 1*, copied from it, and later edits stay there.
    #[test]
    fn an_edit_after_save_opens_after_reveal_1_copied_from_0() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        answer_low(&mut c, a, 2);
        save(&mut c, a).unwrap();
        refused(save(&mut c, a));
        let got = stored(&mut c, a);
        assert_eq!(got.state, AppraisalState::Saved);
        assert!(got.saved_at.is_some() && got.revealed_at.is_some());
        assert!(got.versions[0].frozen_at.is_some());
        edit(&mut c, a, &answer(Domain::D1, None, "1.2", Answer::N)).unwrap();
        edit(&mut c, a, &answer(Domain::D1, None, "1.3", Answer::Py)).unwrap();
        let got = stored(&mut c, a);
        assert_eq!(got.versions.len(), 2);
        let (v0, v1) = (&got.versions[0], &got.versions[1]);
        assert_eq!(answer_in(v0, Domain::D1, None, "1.2"), Some(Answer::Y));
        assert_eq!(answer_in(v0, Domain::D1, None, "1.3"), Some(Answer::N));
        assert_eq!(v1.number, 1);
        assert!(v1.frozen_at.is_none() && v1.closed_at.is_none());
        assert_eq!(answer_in(v1, Domain::D1, None, "1.2"), Some(Answer::N));
        assert_eq!(answer_in(v1, Domain::D1, None, "1.3"), Some(Answer::Py));
        assert_eq!(answered(v1).len(), answered(v0).len());
        assert_eq!(answered(v1)[3..], answered(v0)[3..]);
    }

    /// *Done revising* closes the open revision; the next edit opens *after
    /// reveal 2*, copied from 1. With no revision open, closing is refused.
    #[test]
    fn close_revision_then_edit_opens_after_reveal_2() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        answer_low(&mut c, a, 2);
        refused(close_revision(&mut c, a));
        save(&mut c, a).unwrap();
        refused(close_revision(&mut c, a));
        edit(&mut c, a, &answer(Domain::D1, None, "1.2", Answer::N)).unwrap();
        close_revision(&mut c, a).unwrap();
        refused(close_revision(&mut c, a));
        edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Pn)).unwrap();
        let got = stored(&mut c, a);
        assert_eq!(got.versions.len(), 3);
        let v1 = &got.versions[1];
        assert!(v1.closed_at.is_some());
        assert_eq!(answer_in(v1, Domain::D1, None, "1.1"), Some(Answer::Y));
        assert_eq!(answer_in(v1, Domain::D1, None, "1.2"), Some(Answer::N));
        let v2 = &got.versions[2];
        assert_eq!(v2.number, 2);
        assert_eq!(answer_in(v2, Domain::D1, None, "1.1"), Some(Answer::Pn));
        assert_eq!(answer_in(v2, Domain::D1, None, "1.2"), Some(Answer::N));
        assert_eq!(answered(v2).len(), answered(v1).len());
    }

    /// A frozen or closed version takes no write: an unsaved appraisal whose
    /// first record is frozen (damage) refuses every write as corrupt and
    /// keeps its rows, and the write guard refuses both kinds of version.
    #[test]
    fn a_frozen_version_is_never_written() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
        let v0 = stored(&mut c, a).versions[0].id;
        sql(
            &mut c,
            "UPDATE evidence_appraisal_versions SET frozen_at = datetime('now')",
        );
        corrupt(edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::N)));
        corrupt(refresh_editable(&mut c, a));
        corrupt(check_writable(&mut c, v0));
        let kept = fetch_one(&mut c, "SELECT answer FROM evidence_appraisal_answers", &[])
            .unwrap()
            .unwrap();
        assert_eq!(kept.get_str("answer").unwrap(), "Y");
        sql(
            &mut c,
            "UPDATE evidence_appraisal_versions SET frozen_at = NULL, closed_at = datetime('now')",
        );
        corrupt(check_writable(&mut c, v0));
    }

    /// *Show the machine's verdict now* marks the appraisal not blind and
    /// revealed, freezes nothing, and is taken again; after a save it is
    /// refused, and the save keeps the reveal's time.
    #[test]
    fn reveal_now_marks_not_blind_and_freezes_nothing() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        assert!(stored(&mut c, a).blind);
        answer_low(&mut c, a, 2);
        reveal_now(&mut c, a).unwrap();
        let got = stored(&mut c, a);
        assert!(!got.blind);
        assert!(got.revealed_at.is_some() && got.saved_at.is_none());
        assert_eq!(got.state, AppraisalState::Answering);
        assert!(got.versions[0].frozen_at.is_none());
        sql(
            &mut c,
            "UPDATE evidence_appraisals SET revealed_at = '2026-01-01 00:00:00'",
        );
        reveal_now(&mut c, a).unwrap();
        edit(&mut c, a, &answer(Domain::D1, None, "1.2", Answer::N)).unwrap();
        assert_eq!(stored(&mut c, a).versions.len(), 1);
        save(&mut c, a).unwrap();
        refused(reveal_now(&mut c, a));
        let got = stored(&mut c, a);
        assert_eq!(got.revealed_at.as_deref(), Some("2026-01-01 00:00:00"));
        assert!(!got.blind);
    }

    /// An editable version made under earlier rules takes the current ones;
    /// a frozen one keeps its own.
    #[test]
    fn refresh_editable_updates_only_an_editable_version() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        assert!(!refresh_editable(&mut c, a).unwrap());
        sql(
            &mut c,
            "UPDATE evidence_appraisal_versions SET algorithm_version = 'rob2-old'",
        );
        assert!(stored(&mut c, a).versions[0].earlier_rules);
        assert!(refresh_editable(&mut c, a).unwrap());
        assert!(!stored(&mut c, a).versions[0].earlier_rules);
        answer_low(&mut c, a, 2);
        save(&mut c, a).unwrap();
        sql(
            &mut c,
            "UPDATE evidence_appraisal_versions SET ladder_version = 'ladder/0'",
        );
        assert!(!refresh_editable(&mut c, a).unwrap());
        assert!(stored(&mut c, a).versions[0].earlier_rules);
    }

    /// A row the store did not write is read back as corrupt: version
    /// numbers that are not 0, 1, 2… (two first records the schema's unique
    /// key refuses outright), an answer naming no question, D1 naming an
    /// outcome, D2 naming one not confirmed, and a mark past the text's end.
    #[test]
    fn a_damaged_row_is_corrupt() {
        let damages = [
            "UPDATE evidence_appraisal_versions SET number = 1",
            "UPDATE evidence_appraisal_answers SET question_id = ''",
            "UPDATE evidence_appraisal_answers SET outcome = 0",
            "UPDATE evidence_appraisal_answers SET domain = 'D2', outcome = 2, question_id = '2.1'",
            "UPDATE evidence_appraisal_answers SET mark_end = 100000",
        ];
        for damage in damages {
            let mut c = open_memory_store().unwrap();
            let (_, a, _) = confirmed(&mut c);
            edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
            edit(&mut c, a, &mark("1.1", Some(TextSpan { start: 0, end: 4 }))).unwrap();
            sql(&mut c, damage);
            corrupt(appraisal(&mut c, a));
            corrupt(edit(&mut c, a, &answer(Domain::D1, None, "1.2", Answer::Y)));
        }
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        let second = execute(
            &mut c,
            "INSERT INTO evidence_appraisal_versions (appraisal_id, number, algorithm_version,
                                                      ladder_version)
             VALUES (?, 0, 'a', 'l')",
            &[Value::from(a)],
        );
        assert!(second.is_err());
    }

    /// Deleting the paper deletes its appraisals, every row under them, and
    /// their paired runs.
    #[test]
    fn deleting_paper_cascades() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        for e in [
            answer(Domain::D1, None, "1.1", Answer::Y),
            AppraisalEdit::Note {
                domain: Domain::D1,
                outcome: None,
                note: Some("n".into()),
            },
            AppraisalEdit::Escalate {
                outcome: 0,
                escalate: Some(false),
            },
        ] {
            edit(&mut c, a, &e).unwrap();
        }
        for table in TABLES.iter().chain(&["evidence_runs"]) {
            assert!(count(&mut c, table) > 0, "{table}");
        }
        sql(&mut c, "DELETE FROM papers");
        for table in TABLES.iter().chain(&["evidence_runs"]) {
            assert_eq!(count(&mut c, table), 0, "{table}");
        }
    }

    /// Only a marked span makes a text an appraisal points into: an answer
    /// alone does not, and a cleared mark no longer does.
    #[test]
    fn has_marks_counts_marks_only() {
        let mut c = open_memory_store().unwrap();
        let (cid, a, _) = confirmed(&mut c);
        assert!(!has_marks(&mut c, cid).unwrap());
        edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
        assert!(!has_marks(&mut c, cid).unwrap());
        edit(&mut c, a, &mark("1.1", Some(TextSpan { start: 0, end: 4 }))).unwrap();
        assert!(has_marks(&mut c, cid).unwrap());
        assert!(!has_marks(&mut c, cid + 1).unwrap());
        edit(&mut c, a, &mark("1.1", None)).unwrap();
        assert!(!has_marks(&mut c, cid).unwrap());
    }

    /// The webview's JSON reads as the edit and the input it means.
    #[test]
    fn edits_and_inputs_read_from_the_webview_s_json() {
        let e: AppraisalEdit = serde_json::from_str(
            r#"{"kind":"answer","domain":"D2","outcome":0,"question_id":"2.1","answer":"PY"}"#,
        )
        .unwrap();
        assert_eq!(e, answer(Domain::D2, Some(0), "2.1", Answer::Py));
        let e: AppraisalEdit =
            serde_json::from_str(r#"{"kind":"overall_override","outcome":1,"judgement":"high"}"#)
                .unwrap();
        assert_eq!(
            e,
            AppraisalEdit::OverallOverride {
                outcome: 1,
                judgement: Some(Judgement::High)
            }
        );
        let i: ConfirmInput = serde_json::from_str(
            r#"{"design":"rct","rct_layout":"parallel","outcomes":[{"from_proposed":null,
                "name":"Pain","timepoint":null,"result":null}]}"#,
        )
        .unwrap();
        assert_eq!(i.outcomes[0].name, "Pain");
    }

    /// D3 of outcome 0 answered so all four questions apply, high.
    fn d3_all_asked(c: &mut Connection, a: i64) {
        for (q, x) in [
            ("3.1", Answer::N),
            ("3.2", Answer::N),
            ("3.3", Answer::Y),
            ("3.4", Answer::Y),
        ] {
            edit(c, a, &answer(Domain::D3, Some(0), q, x)).unwrap();
        }
    }

    /// An answer that closes later questions makes the derived verdict follow
    /// at once; the later answers stay stored, not counted.
    #[test]
    fn an_inapplicable_answer_is_kept_and_not_counted() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        d3_all_asked(&mut c, a);
        let v0 = stored(&mut c, a).versions.remove(0);
        assert_eq!(derived_in(&v0, Domain::D3, Some(0)), Some(Judgement::High));
        edit(&mut c, a, &answer(Domain::D3, Some(0), "3.1", Answer::Y)).unwrap();
        let v0 = stored(&mut c, a).versions.remove(0);
        assert_eq!(derived_in(&v0, Domain::D3, Some(0)), Some(Judgement::Low));
        for q in ["3.2", "3.3", "3.4"] {
            assert!(answer_in(&v0, Domain::D3, Some(0), q).is_some(), "{q}");
        }
    }

    /// An answer kept while its question did not apply counts again once it
    /// applies again.
    #[test]
    fn it_counts_again_when_it_applies_again() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        d3_all_asked(&mut c, a);
        edit(&mut c, a, &answer(Domain::D3, Some(0), "3.1", Answer::Y)).unwrap();
        edit(&mut c, a, &answer(Domain::D3, Some(0), "3.1", Answer::Pn)).unwrap();
        let v0 = stored(&mut c, a).versions.remove(0);
        assert_eq!(derived_in(&v0, Domain::D3, Some(0)), Some(Judgement::High));
    }

    /// NI on 3.2, which RoB 2 does not allow, is refused and nothing is
    /// written; the other answers stand.
    #[test]
    fn ni_on_3_2_is_refused() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        edit(&mut c, a, &answer(Domain::D3, Some(0), "3.1", Answer::N)).unwrap();
        refused(edit(
            &mut c,
            a,
            &answer(Domain::D3, Some(0), "3.2", Answer::Ni),
        ));
        let v0 = stored(&mut c, a).versions.remove(0);
        assert_eq!(answered(&v0), vec![("3.1".into(), Some(Answer::N))]);
        assert!(v0.domains.is_empty() && v0.overalls.is_empty());
    }

    /// A question bmlib does not know, or one of another domain, sent by a
    /// tampered webview, is refused and nothing is written.
    #[test]
    fn an_unknown_question_is_refused() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        for (d, o, q) in [
            (Domain::D1, None, "1.9"),
            (Domain::D1, None, "2.1"),
            (Domain::D4, Some(1), "4.6"),
            (Domain::D2, Some(0), ""),
        ] {
            refused(edit(&mut c, a, &answer(d, o, q, Answer::Y)));
            refused(edit(&mut c, a, &mark_in(d, o, q)));
        }
        for table in &TABLES[3..] {
            assert_eq!(count(&mut c, table), 0, "{table}");
        }
    }

    /// A mark on question `q` of `domain` for `outcome`.
    fn mark_in(domain: Domain, outcome: Option<usize>, q: &str) -> AppraisalEdit {
        AppraisalEdit::Mark {
            domain,
            outcome,
            question_id: q.into(),
            span: Some(TextSpan { start: 0, end: 4 }),
        }
    }

    /// The reader's override of D2 for outcome 0.
    fn d2_high() -> AppraisalEdit {
        AppraisalEdit::Override {
            domain: Domain::D2,
            outcome: Some(0),
            judgement: Some(Judgement::High),
        }
    }

    /// A low first record gives each outcome a low overall and a ladder from
    /// it; D2 overridden to high makes that outcome's overall high and its
    /// ladder start from high, the other outcome unchanged.
    #[test]
    fn an_override_feeds_the_overall_and_the_ladder() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        answer_low(&mut c, a, 2);
        let v0 = stored(&mut c, a).versions.remove(0);
        for o in 0..2 {
            let row = overall_in(&v0, o);
            assert_eq!(row.derived, Some(Judgement::Low));
            assert_eq!(row.ladder_start.as_deref(), Some("high"));
            // Low risk of bias costs nothing; 312 analysed is a small trial.
            assert_eq!(row.ladder_level.as_deref(), Some("moderate"));
            assert!(row.ladder_error.is_none());
        }
        edit(&mut c, a, &d2_high()).unwrap();
        let v0 = stored(&mut c, a).versions.remove(0);
        assert_eq!(derived_in(&v0, Domain::D2, Some(0)), Some(Judgement::Low));
        let high = overall_in(&v0, 0);
        assert_eq!(high.derived, Some(Judgement::High));
        assert_eq!(high.ladder_level.as_deref(), Some("very_low"));
        assert!(high.ladder_weight.is_some());
        let steps: serde_json::Value =
            serde_json::from_str(high.ladder_steps_json.as_deref().unwrap()).unwrap();
        assert_eq!(
            steps[0],
            serde_json::json!({"reason": "risk_of_bias", "judgement": "high", "n": null,
                               "threshold": null, "levels": 2})
        );
        assert_eq!(steps[1]["reason"], "small_trial");
        assert_eq!(overall_in(&v0, 1).derived, Some(Judgement::Low));
        assert_eq!(overall_in(&v0, 1).ladder_level.as_deref(), Some("moderate"));
    }

    /// That override is exactly one departure, at D2, and none at the
    /// overall verdict it feeds.
    #[test]
    fn one_override_is_one_departure() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        answer_low(&mut c, a, 2);
        assert!(departures(&rater_record(&stored(&mut c, a).versions[0])).is_empty());
        edit(&mut c, a, &d2_high()).unwrap();
        let record = rater_record(&stored(&mut c, a).versions[0]);
        assert_eq!(
            departures(&record),
            vec![Departure {
                scope: DepartureScope::Domain(Domain::D2, Some(0)),
                given: Judgement::High,
                rules: Judgement::Low,
            }]
        );
    }

    /// A D1 edit re-derives every outcome's overall verdict and ladder.
    #[test]
    fn d1_edits_rederive_every_outcome() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        answer_low(&mut c, a, 2);
        edit(&mut c, a, &answer(Domain::D1, None, "1.2", Answer::N)).unwrap();
        let v0 = stored(&mut c, a).versions.remove(0);
        assert_eq!(derived_in(&v0, Domain::D1, None), Some(Judgement::High));
        for o in 0..2 {
            assert_eq!(overall_in(&v0, o).derived, Some(Judgement::High), "{o}");
            assert_eq!(overall_in(&v0, o).ladder_level.as_deref(), Some("very_low"));
        }
    }

    /// Save needs every applicable question answered (a mark alone is no
    /// answer) in every outcome, and the escalation where the verdicts as
    /// left ask it.
    #[test]
    fn save_needs_a_complete_first_record() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        answer_low(&mut c, a, 1);
        refused(save(&mut c, a));
        answer_low(&mut c, a, 2);
        let clear = AppraisalEdit::Answer {
            domain: Domain::D1,
            outcome: None,
            question_id: "1.1".into(),
            answer: None,
        };
        edit(&mut c, a, &mark("1.1", Some(TextSpan { start: 0, end: 4 }))).unwrap();
        edit(&mut c, a, &clear).unwrap();
        refused(save(&mut c, a));
        edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
        for d in [Domain::D2, Domain::D3] {
            let some = AppraisalEdit::Override {
                domain: d,
                outcome: Some(0),
                judgement: Some(Judgement::SomeConcerns),
            };
            edit(&mut c, a, &some).unwrap();
        }
        assert_eq!(overall_in(&stored(&mut c, a).versions[0], 0).derived, None);
        refused(save(&mut c, a));
        let escalate = AppraisalEdit::Escalate {
            outcome: 0,
            escalate: Some(false),
        };
        edit(&mut c, a, &escalate).unwrap();
        let v0 = stored(&mut c, a).versions.remove(0);
        assert_eq!(overall_in(&v0, 0).derived, Some(Judgement::SomeConcerns));
        save(&mut c, a).unwrap();
    }

    /// A first record frozen while not complete (not this code's doing) is
    /// read back as corrupt.
    #[test]
    fn a_frozen_incomplete_version_is_corrupt() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
        sql(
            &mut c,
            "UPDATE evidence_appraisal_versions SET frozen_at = datetime('now')",
        );
        sql(
            &mut c,
            "UPDATE evidence_appraisals SET saved_at = datetime('now'),
                                            revealed_at = datetime('now')",
        );
        corrupt(appraisal(&mut c, a));
    }

    /// Damages a version's derived verdicts and ladder in SQL, as rules
    /// other than today's could have left them.
    fn derived_high(c: &mut Connection) {
        sql(c, "UPDATE evidence_appraisal_domains SET derived = 'high'");
        sql(
            c,
            "UPDATE evidence_appraisal_overalls SET derived = 'high', ladder_level = 'very_low'",
        );
    }

    /// An unsaved first record made under earlier rules is re-derived under
    /// the current ones when read for editing, and takes their versions.
    #[test]
    fn an_editable_version_rederives_under_current_rules() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        answer_low(&mut c, a, 2);
        derived_high(&mut c);
        sql(
            &mut c,
            "UPDATE evidence_appraisal_versions SET algorithm_version = 'rob2-old'",
        );
        assert!(refresh_editable(&mut c, a).unwrap());
        let v0 = stored(&mut c, a).versions.remove(0);
        assert!(!v0.earlier_rules);
        assert_eq!(derived_in(&v0, Domain::D1, None), Some(Judgement::Low));
        assert_eq!(derived_in(&v0, Domain::D4, Some(1)), Some(Judgement::Low));
        assert_eq!(overall_in(&v0, 1).derived, Some(Judgement::Low));
        assert_eq!(overall_in(&v0, 1).ladder_level.as_deref(), Some("moderate"));
    }

    /// A frozen first record made under earlier rules keeps its rules and
    /// its stored verdicts; a revision copied from it is derived under the
    /// current rules, leaving the first record as it was.
    #[test]
    fn a_frozen_version_keeps_its_rules() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        answer_low(&mut c, a, 2);
        save(&mut c, a).unwrap();
        derived_high(&mut c);
        sql(
            &mut c,
            "UPDATE evidence_appraisal_versions SET algorithm_version = 'rob2-old'",
        );
        assert!(!refresh_editable(&mut c, a).unwrap());
        let before = stored(&mut c, a).versions.remove(0);
        assert!(before.earlier_rules);
        assert_eq!(derived_in(&before, Domain::D1, None), Some(Judgement::High));
        assert_eq!(overall_in(&before, 0).derived, Some(Judgement::High));
        let note = AppraisalEdit::Note {
            domain: Domain::D1,
            outcome: None,
            note: Some("Checked.".into()),
        };
        edit(&mut c, a, &note).unwrap();
        let got = stored(&mut c, a);
        assert_eq!(got.versions[0], before);
        let v1 = &got.versions[1];
        assert!(!v1.earlier_rules);
        assert_eq!(derived_in(v1, Domain::D1, None), Some(Judgement::Low));
        assert_eq!(overall_in(v1, 0).derived, Some(Judgement::Low));
    }

    /// After a re-pin that drops a question, the reader's answer to it is
    /// kept on every version, read back, never counted, and blocks nothing:
    /// the draft re-derives, saves and reads back.
    #[test]
    fn a_question_bmlib_dropped_is_kept_and_not_counted() {
        let mut c = open_memory_store().unwrap();
        let (_, _, a, _) = proposed(&mut c);
        confirm(&mut c, a, &input(1)).unwrap();
        answer_low(&mut c, a, 1);
        let v0 = stored(&mut c, a).versions[0].id;
        execute(
            &mut c,
            "INSERT INTO evidence_appraisal_answers (version_id, domain, question_id, answer)
             VALUES (?, 'D1', '1.9', 'NI')",
            &[Value::from(v0)],
        )
        .unwrap();
        sql(
            &mut c,
            "UPDATE evidence_appraisal_versions SET algorithm_version = 'rob2-old'",
        );
        assert!(refresh_editable(&mut c, a).unwrap());
        let got = stored(&mut c, a).versions.remove(0);
        assert_eq!(answer_in(&got, Domain::D1, None, "1.9"), Some(Answer::Ni));
        assert_eq!(derived_in(&got, Domain::D1, None), Some(Judgement::Low));
        assert!(!rater_record(&got).domains[0].answers.contains_key("1.9"));
        save(&mut c, a).unwrap();
        edit(&mut c, a, &answer(Domain::D1, None, "1.3", Answer::Y)).unwrap();
        let got = stored(&mut c, a);
        assert_eq!(
            answer_in(&got.versions[1], Domain::D1, None, "1.9"),
            Some(Answer::Ni)
        );
        assert_eq!(count(&mut c, "evidence_appraisal_answers"), 2 * 14);
    }
}
