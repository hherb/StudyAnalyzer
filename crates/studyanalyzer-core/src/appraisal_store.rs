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

use std::collections::BTreeSet;

use bmlib::db::{execute, fetch_all, fetch_one, transaction_with, Db, Row, Value};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::quality::{
    allowed_answers, questions, start_level, study_design_from_str, Answer, Domain, Judgement,
    RctLayout, ReportedResult, StudyDesign, LADDER_VERSION, ROB2_ALGORITHM_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::config::APPRAISAL_NOTE_MAX_CHARS;
use crate::evidence_rows::{from_wire, wire};
use crate::evidence_store::{domain_name, domain_of, plan_paired, StepStatus, STEP_OUTCOMES};
use crate::pico_store::{optional, text_of};
use crate::store::{canonical_by_id, last_id, latest_canonical_id, PaperId, StoreError};

/// Where an appraisal stands, derived from its row: never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppraisalState {
    /// The design and outcomes are not confirmed yet (no `confirmed_at`).
    Confirming,
    /// Confirmed on a covered design, not saved: the reader answers.
    Answering,
    /// Saved: the first record is frozen, and compare is open.
    Saved,
    /// Confirmed on a design RoB 2's parallel-group questions do not cover
    /// (no ladder start, or a layout `RctLayout::is_covered` refuses).
    NotCovered,
}

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

/// One confirmed outcome, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmedOutcome {
    /// Its row id.
    pub id: i64,
    /// Its index among the confirmed outcomes, as D2–D5 rows name it.
    pub position: usize,
    /// The machine's proposed outcome it came from, `None` when added.
    pub from_proposed: Option<usize>,
    /// Its name.
    pub name: String,
    /// When it was measured.
    pub timepoint: Option<String>,
    /// Its reported effect, held to `ReportedResult::validate`.
    pub result: Option<ReportedResult>,
}

/// One stored answer row of a version: what the reader picked, marked and
/// ticked for one question. A row may hold only a mark or a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct ReaderAnswer {
    /// The question's domain.
    pub domain: Domain,
    /// The confirmed outcome, `None` for D1.
    pub outcome: Option<usize>,
    /// RoB 2's question number.
    pub question_id: String,
    /// The answer picked, if any (never *not applicable*: that is derived).
    pub answer: Option<Answer>,
    /// The marked span, in code points, inside the appraisal's text.
    pub mark: Option<TextSpan>,
    /// *I also used: protocol*.
    pub protocol: bool,
    /// *I also used: registry*.
    pub registry: bool,
    /// *I also used: supplement*.
    pub supplement: bool,
}

/// One stored domain row of a version: its derived verdict, the reader's
/// override and note.
#[derive(Debug, Clone, PartialEq)]
pub struct ReaderDomain {
    /// The domain.
    pub domain: Domain,
    /// The confirmed outcome, `None` for D1.
    pub outcome: Option<usize>,
    /// The verdict RoB 2's algorithm gives the answers, once all are in.
    pub derived: Option<Judgement>,
    /// The reader's own verdict.
    pub override_: Option<Judgement>,
    /// The reader's note.
    pub note: Option<String>,
}

/// One stored overall row of a version: one outcome's escalation answer,
/// overall verdicts and the reader's ladder, as derived and stored.
#[derive(Debug, Clone, PartialEq)]
pub struct ReaderOverall {
    /// The confirmed outcome.
    pub outcome: usize,
    /// The answer to the escalation question, if asked and answered.
    pub escalate: Option<bool>,
    /// The overall verdict RoB 2's algorithm gives.
    pub derived: Option<Judgement>,
    /// The reader's own overall verdict.
    pub override_: Option<Judgement>,
    /// The ladder's start level, as bmlib names it.
    pub ladder_start: Option<String>,
    /// The ladder's end level, as bmlib names it.
    pub ladder_level: Option<String>,
    /// The ladder's number.
    pub ladder_weight: Option<f64>,
    /// Why no ladder was built.
    pub ladder_error: Option<String>,
    /// The ladder's steps, as JSON shaped as `LadderView.steps`.
    pub ladder_steps_json: Option<String>,
}

/// One version of an appraisal: the first record (number 0) or a revision.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredVersion {
    /// Its row id.
    pub id: i64,
    /// 0 for the first record, then 1, 2, … for each *after reveal*.
    pub number: u32,
    /// RoB 2's rules it was derived under (`ROB2_ALGORITHM_VERSION`).
    pub algorithm_version: String,
    /// The ladder's rules it was derived under (`LADDER_VERSION`).
    pub ladder_version: String,
    /// Whether those are not the pinned bmlib's.
    pub earlier_rules: bool,
    /// When it was made (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// When the first record was saved; never set on a revision.
    pub frozen_at: Option<String>,
    /// When a revision was closed (*Done revising*); never set on version 0.
    pub closed_at: Option<String>,
    /// Its answer rows, by domain, outcome and question.
    pub answers: Vec<ReaderAnswer>,
    /// Its domain rows, by domain and outcome.
    pub domains: Vec<ReaderDomain>,
    /// Its overall rows, by outcome.
    pub overalls: Vec<ReaderOverall>,
}

/// A stored appraisal, as [`appraisal`] reads it back, its shape checked.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredAppraisal {
    /// Its row id.
    pub id: i64,
    /// The paper.
    pub paper: PaperId,
    /// The canonical text it is on.
    pub canonical_id: i64,
    /// That text, which every mark falls inside.
    pub text: CanonicalText,
    /// Whether it is blind: started with no run shown, and not revealed
    /// before its save.
    pub blind: bool,
    /// When it was started (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub started_at: String,
    /// When the design and outcomes were confirmed.
    pub confirmed_at: Option<String>,
    /// When the machine's verdict was first shown (a save or a reveal).
    pub revealed_at: Option<String>,
    /// When the first record was saved.
    pub saved_at: Option<String>,
    /// When it was set aside.
    pub set_aside_at: Option<String>,
    /// The confirmed design's bmlib name.
    pub design: Option<String>,
    /// The confirmed layout's bmlib name.
    pub rct_layout: Option<String>,
    /// Where it stands.
    pub state: AppraisalState,
    /// The confirmed outcomes, by position.
    pub outcomes: Vec<ConfirmedOutcome>,
    /// Its versions, by number.
    pub versions: Vec<StoredVersion>,
}

/// The table of a version's answer rows.
const ANSWERS: &str = "evidence_appraisal_answers";
/// The table of a version's domain rows.
const DOMAINS: &str = "evidence_appraisal_domains";
/// The table of a version's overall rows.
const OVERALLS: &str = "evidence_appraisal_overalls";

/// An answer row's columns but its id and version, as copied and read.
const ANSWER_COLUMNS: &str = "domain, outcome, question_id, answer, mark_start, mark_end, \
    used_protocol, used_registry, used_supplement";
/// A domain row's columns but its id and version.
const DOMAIN_COLUMNS: &str = "domain, outcome, derived, override, note";
/// An overall row's columns but its id and version.
const OVERALL_COLUMNS: &str = "outcome, escalate, derived, override, ladder_start, ladder_level, \
    ladder_weight, ladder_error, ladder_steps_json";

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
            return Err(StoreError::Invalid(format!(
                "paper {} already has an open appraisal; set it aside to start another",
                paper.as_str()
            )));
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
            return Err(invalid(appraisal_id, "it was set aside"));
        }
        if a.confirmed_at.is_some() {
            return Err(invalid(
                appraisal_id,
                "its design and outcomes are confirmed already; set it aside to start again",
            ));
        }
        let covered = design_covered(&input.design, input.rct_layout.as_deref())
            .map_err(|why| invalid(appraisal_id, why))?;
        if covered && input.outcomes.is_empty() {
            return Err(invalid(
                appraisal_id,
                "at least one primary outcome is needed",
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
        let version_id = match open_version(&a) {
            Some(v) => v.id,
            None => new_revision(tx, &a)?,
        };
        check_writable(tx, version_id)?;
        apply(tx, version_id, edit)?;
        rederive(tx, version_id)
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
            return Err(invalid(appraisal_id, "its first record is saved already"));
        }
        let first = a
            .versions
            .first()
            .ok_or_else(|| no_first_record(appraisal_id))?;
        check_writable(tx, first.id)?;
        if !version_complete(tx, first.id)? {
            return Err(invalid(
                appraisal_id,
                "answer every question that applies, and each escalation asked, before saving",
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
            return Err(invalid(appraisal_id, "it was set aside"));
        }
        if a.saved_at.is_some() {
            return Err(invalid(
                appraisal_id,
                "it is saved, so the machine's verdict is shown already",
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
            return Err(invalid(appraisal_id, "it was set aside"));
        }
        let Some(v) = open_version(&a).filter(|v| v.number > 0) else {
            return Err(invalid(appraisal_id, "no revision is open"));
        };
        check_writable(tx, v.id)?;
        execute(
            tx,
            "UPDATE evidence_appraisal_versions SET closed_at = datetime('now') WHERE id = ?",
            &[Value::from(v.id)],
        )?;
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
            return Err(invalid(appraisal_id, "it was set aside already"));
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
        rederive(tx, v.id)?;
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

/// The stored appraisal `appraisal_id`, or `None` if there is none: its row,
/// confirmed outcomes by position, and versions by number with their rows.
/// A row this code did not write is [`StoreError::Corrupt`]: a missing text,
/// a confirmation half stored or on a design bmlib does not know, versions
/// that are not 0, 1, 2… or with an open one before the last, a first record
/// frozen without a save (or the reverse), a question not in
/// `questions(domain)` on a version under the current rules, D1 naming an
/// outcome or D2–D5 naming none or one not confirmed, a mark outside the
/// text, or a result `ReportedResult::validate` refuses.
pub fn appraisal(
    db: &mut dyn Db,
    appraisal_id: i64,
) -> Result<Option<StoredAppraisal>, StoreError> {
    let Some(r) = fetch_one(
        db,
        "SELECT paper_id, canonical_id, blind, started_at, confirmed_at, revealed_at, saved_at,
                set_aside_at, design, rct_layout
         FROM evidence_appraisals WHERE id = ?",
        &[Value::from(appraisal_id)],
    )?
    else {
        return Ok(None);
    };
    let bad = |why: String| StoreError::Corrupt(format!("appraisal {appraisal_id}: {why}"));
    let paper = PaperId(r.get_str("paper_id")?.to_string());
    let canonical_id = r.get_i64("canonical_id")?;
    let text = canonical_by_id(db, &paper, canonical_id)?
        .ok_or_else(|| bad("its canonical text is not stored".into()))?
        .text;
    let opt = |name: &str| optional(&r, name, text_of).map_err(bad);
    let (confirmed_at, revealed_at) = (opt("confirmed_at")?, opt("revealed_at")?);
    let (saved_at, set_aside_at) = (opt("saved_at")?, opt("set_aside_at")?);
    let (design, rct_layout) = (opt("design")?, opt("rct_layout")?);
    let covered = match (&confirmed_at, &design) {
        (None, None) if rct_layout.is_none() => None,
        (Some(_), Some(d)) => Some(design_covered(d, rct_layout.as_deref()).map_err(bad)?),
        _ => return Err(bad("its confirmation and design are half stored".into())),
    };
    let outcomes = confirmed_outcomes(db, appraisal_id)?;
    if covered.is_none() && !outcomes.is_empty() {
        return Err(bad("it holds outcomes, but is not confirmed".into()));
    }
    if covered == Some(true) && outcomes.is_empty() {
        return Err(bad(
            "it is confirmed on a covered design with no outcome".into()
        ));
    }
    let state = match (covered, &saved_at) {
        (None, None) => AppraisalState::Confirming,
        (Some(false), None) => AppraisalState::NotCovered,
        (Some(true), None) => AppraisalState::Answering,
        (Some(true), Some(_)) => AppraisalState::Saved,
        (_, Some(_)) => return Err(bad("it is saved, but not on a covered design".into())),
    };
    let versions = versions(db, outcomes.len(), &text, appraisal_id)?;
    check_versions(&versions, saved_at.is_some()).map_err(bad)?;
    Ok(Some(StoredAppraisal {
        id: appraisal_id,
        paper,
        canonical_id,
        text,
        blind: r.get_i64("blind")? != 0,
        started_at: r.get_str("started_at")?.to_string(),
        confirmed_at,
        revealed_at,
        saved_at,
        set_aside_at,
        design,
        rct_layout,
        state,
        outcomes,
        versions,
    }))
}

/// Whether an appraisal on canonical text `canonical_id` holds a marked
/// span, so the text must not be re-derived or replaced. Answers alone are
/// no dependent.
pub fn has_marks(db: &mut dyn Db, canonical_id: i64) -> Result<bool, StoreError> {
    Ok(fetch_one(
        db,
        "SELECT 1 AS one FROM evidence_appraisal_answers x
         JOIN evidence_appraisal_versions v ON v.id = x.version_id
         JOIN evidence_appraisals a ON a.id = v.appraisal_id
         WHERE a.canonical_id = ? AND x.mark_start IS NOT NULL LIMIT 1",
        &[Value::from(canonical_id)],
    )?
    .is_some())
}

/// Re-derives version `version_id`'s verdicts, overalls and ladders from its
/// rows and stores them, inside the caller's transaction. Every write to a
/// version calls it.
fn rederive(_tx: &mut dyn Db, _version_id: i64) -> Result<(), StoreError> {
    // Task 4 fills this in.
    Ok(())
}

/// Whether version `version_id` is complete: every applicable question of
/// D1 and of each outcome's D2–D5 answered, and each needed escalation.
fn version_complete(_tx: &mut dyn Db, _version_id: i64) -> Result<bool, StoreError> {
    // Task 4 fills this in.
    Ok(true)
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

/// A refusal of a request on appraisal `appraisal_id`, saying `why`.
fn invalid(appraisal_id: i64, why: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(format!("appraisal {appraisal_id}: {why}"))
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
        return Err(invalid(appraisal_id, "it was set aside"));
    }
    match a.state {
        AppraisalState::Confirming => Err(invalid(
            appraisal_id,
            "confirm its design and outcomes first",
        )),
        AppraisalState::NotCovered => Err(invalid(
            appraisal_id,
            format!(
                "no method for this design yet: {}",
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
/// version's rows under the current rules, and returns its id.
fn new_revision(db: &mut dyn Db, a: &StoredAppraisal) -> Result<i64, StoreError> {
    let latest = a.versions.last().ok_or_else(|| no_first_record(a.id))?;
    let id = insert_version(db, a.id, latest.number + 1)?;
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
        latest.number + 1,
        latest.number
    );
    Ok(id)
}

/// Whether `design` with `layout` is one RoB 2's parallel-group questions
/// cover, as `assess_unattended` decides: a ladder start (`start_level`) and
/// a covered layout. A design or layout bmlib does not know, a randomised
/// trial without a layout, or a layout on any other design is an error.
fn design_covered(design: &str, layout: Option<&str>) -> Result<bool, String> {
    let d = study_design_from_str(design);
    if d.as_str() != design {
        return Err(format!("{design:?} is not a design"));
    }
    let layout = match (d == StudyDesign::Rct, layout) {
        (true, Some(l)) => {
            Some(RctLayout::from_wire(l).ok_or_else(|| format!("{l:?} is not a trial layout"))?)
        }
        (true, None) => return Err("a randomised trial needs its layout".into()),
        (false, Some(l)) => return Err(format!("only a randomised trial has a layout, not {l:?}")),
        (false, None) => None,
    };
    Ok(start_level(d).is_some() && layout.is_some_and(RctLayout::is_covered))
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
            "the machine's design and outcome proposals are not stored yet; continue its run first",
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
            Value::from(o.timepoint.as_deref()),
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

/// Refuses `outcome` unless it is one of the `outcomes` confirmed.
fn check_outcome(outcome: usize, outcomes: usize) -> Result<(), String> {
    if outcome < outcomes {
        return Ok(());
    }
    Err(format!(
        "outcome {outcome} is not one of the {outcomes} confirmed"
    ))
}

/// Refuses a row of `domain` for `outcome` unless D1 names no outcome and
/// D2–D5 one of the `outcomes` confirmed.
fn check_scope(domain: Domain, outcome: Option<usize>, outcomes: usize) -> Result<(), String> {
    match (domain, outcome) {
        (Domain::D1, None) => Ok(()),
        (Domain::D1, Some(o)) => Err(format!("D1 is judged for the study, not for outcome {o}")),
        (_, Some(o)) => check_outcome(o, outcomes),
        (d, None) => Err(format!("{} is judged per outcome", domain_name(d))),
    }
}

/// Refuses question `question_id` of `domain` for `outcome` unless its scope
/// holds ([`check_scope`]) and it is one of `questions(domain)`.
fn check_question(
    domain: Domain,
    outcome: Option<usize>,
    question_id: &str,
    outcomes: usize,
) -> Result<(), String> {
    check_scope(domain, outcome, outcomes)?;
    if questions(domain).contains(&question_id) {
        return Ok(());
    }
    Err(format!(
        "{question_id:?} is not a question of {}",
        domain_name(domain)
    ))
}

/// Refuses `span` unless it is non-empty and inside `text`.
fn check_span(text: &CanonicalText, span: TextSpan) -> Result<(), String> {
    if span.start < span.end && text.span_text(span).is_some() {
        return Ok(());
    }
    Err(format!("the span {span} is not inside the paper's text"))
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

/// The id of version `version_id`'s row of `table` keyed by `key` (column
/// and value pairs, NULL matching NULL), inserted with nothing else set when
/// there is none.
fn keyed_row(
    db: &mut dyn Db,
    table: &str,
    version_id: i64,
    key: &[(&str, Value)],
) -> Result<i64, StoreError> {
    let mut values = vec![Value::from(version_id)];
    values.extend(key.iter().map(|(_, v)| v.clone()));
    let matches: String = key.iter().map(|(c, _)| format!(" AND {c} IS ?")).collect();
    let found = fetch_one(
        db,
        &format!("SELECT id FROM {table} WHERE version_id = ?{matches}"),
        &values,
    )?;
    if let Some(r) = found {
        return Ok(r.get_i64("id")?);
    }
    let columns: Vec<&str> = key.iter().map(|(c, _)| *c).collect();
    let marks = vec!["?"; values.len()].join(", ");
    execute(
        db,
        &format!(
            "INSERT INTO {table} (version_id, {}) VALUES ({marks})",
            columns.join(", ")
        ),
        &values,
    )?;
    last_id(db)
}

/// Sets `assignments` (`"a = ?, b = ?"`) to `values` on row `id` of `table`.
fn set(
    db: &mut dyn Db,
    table: &str,
    id: i64,
    assignments: &str,
    mut values: Vec<Value>,
) -> Result<(), StoreError> {
    values.push(Value::from(id));
    execute(
        db,
        &format!("UPDATE {table} SET {assignments} WHERE id = ?"),
        &values,
    )?;
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
fn apply(db: &mut dyn Db, version_id: i64, edit: &AppraisalEdit) -> Result<(), StoreError> {
    let judgement = |j: &Option<Judgement>| -> Result<Value, StoreError> {
        Ok(Value::from(j.as_ref().map(wire).transpose()?))
    };
    match edit {
        AppraisalEdit::Answer {
            domain,
            outcome,
            question_id,
            answer,
        } => {
            let id = keyed_row(
                db,
                ANSWERS,
                version_id,
                &answer_key(*domain, *outcome, question_id),
            )?;
            let answer = Value::from(answer.as_ref().map(wire).transpose()?);
            set(db, ANSWERS, id, "answer = ?", vec![answer])
        }
        AppraisalEdit::Mark {
            domain,
            outcome,
            question_id,
            span,
        } => {
            let id = keyed_row(
                db,
                ANSWERS,
                version_id,
                &answer_key(*domain, *outcome, question_id),
            )?;
            let ends = vec![
                Value::from(span.map(|s| s.start)),
                Value::from(span.map(|s| s.end)),
            ];
            set(db, ANSWERS, id, "mark_start = ?, mark_end = ?", ends)
        }
        AppraisalEdit::Sources {
            domain,
            outcome,
            question_id,
            protocol,
            registry,
            supplement,
        } => {
            let id = keyed_row(
                db,
                ANSWERS,
                version_id,
                &answer_key(*domain, *outcome, question_id),
            )?;
            let ticks = vec![
                Value::from(*protocol),
                Value::from(*registry),
                Value::from(*supplement),
            ];
            let columns = "used_protocol = ?, used_registry = ?, used_supplement = ?";
            set(db, ANSWERS, id, columns, ticks)
        }
        AppraisalEdit::Override {
            domain,
            outcome,
            judgement: j,
        } => {
            let id = keyed_row(db, DOMAINS, version_id, &domain_key(*domain, *outcome))?;
            set(db, DOMAINS, id, "override = ?", vec![judgement(j)?])
        }
        AppraisalEdit::Note {
            domain,
            outcome,
            note,
        } => {
            let id = keyed_row(db, DOMAINS, version_id, &domain_key(*domain, *outcome))?;
            set(
                db,
                DOMAINS,
                id,
                "note = ?",
                vec![Value::from(note.as_deref())],
            )
        }
        AppraisalEdit::Escalate { outcome, escalate } => {
            let key = [("outcome", Value::from(*outcome))];
            let id = keyed_row(db, OVERALLS, version_id, &key)?;
            set(
                db,
                OVERALLS,
                id,
                "escalate = ?",
                vec![Value::from(*escalate)],
            )
        }
        AppraisalEdit::OverallOverride {
            outcome,
            judgement: j,
        } => {
            let key = [("outcome", Value::from(*outcome))];
            let id = keyed_row(db, OVERALLS, version_id, &key)?;
            set(db, OVERALLS, id, "override = ?", vec![judgement(j)?])
        }
    }
}

/// Appraisal `appraisal_id`'s confirmed outcomes, by position: positions
/// 0, 1, 2…, and a result stored whole and coherent, or not at all.
fn confirmed_outcomes(
    db: &mut dyn Db,
    appraisal_id: i64,
) -> Result<Vec<ConfirmedOutcome>, StoreError> {
    let rows = fetch_all(
        db,
        "SELECT id, position, from_proposed, name, timepoint, measure, scale, estimate, ci_low,
                ci_high, ci_level, n_analysed
         FROM evidence_appraisal_outcomes WHERE appraisal_id = ? ORDER BY position",
        &[Value::from(appraisal_id)],
    )?;
    let mut outcomes = Vec::with_capacity(rows.len());
    for (index, r) in rows.iter().enumerate() {
        let id = r.get_i64("id")?;
        let bad = |why: String| StoreError::Corrupt(format!("appraisal outcome {id}: {why}"));
        outcomes.push(outcome_from(r, index).map_err(bad)?);
    }
    Ok(outcomes)
}

/// Confirmed outcome row `r`, expected at `index`, read back.
fn outcome_from(r: &Row, index: usize) -> Result<ConfirmedOutcome, String> {
    let id = r.get_i64("id").map_err(|e| e.to_string())?;
    let position = r.get_i64("position").map_err(|e| e.to_string())?;
    if usize::try_from(position).ok() != Some(index) {
        return Err(format!("its position is {position}, not {index}"));
    }
    let int = |name: &str| optional(r, name, Value::as_i64);
    let real = |name: &str| optional(r, name, Value::as_f64);
    let text = |name: &str| optional(r, name, text_of);
    let from_proposed = int("from_proposed")?
        .map(|f| usize::try_from(f).map_err(|_| format!("from_proposed {f} is not an index")))
        .transpose()?;
    let result = match (text("measure")?, text("scale")?, real("estimate")?) {
        (None, None, None) => None,
        (Some(measure), Some(scale), Some(estimate)) => {
            let n_analysed = int("n_analysed")?
                .map(|n| u32::try_from(n).map_err(|_| format!("n_analysed {n} is not a count")))
                .transpose()?;
            let result = ReportedResult {
                measure,
                scale: from_wire(&scale)?,
                estimate,
                ci_low: real("ci_low")?,
                ci_high: real("ci_high")?,
                ci_level: real("ci_level")?,
                n_analysed,
            };
            result.validate()?;
            Some(result)
        }
        _ => return Err("its result is half stored".into()),
    };
    Ok(ConfirmedOutcome {
        id,
        position: index,
        from_proposed,
        name: r.get_str("name").map_err(|e| e.to_string())?.to_string(),
        timepoint: text("timepoint")?,
        result,
    })
}

/// Appraisal `appraisal_id`'s versions by number, each with its rows read
/// back against its `outcomes` confirmed outcomes and its `text`. Numbers
/// must be 0, 1, 2…: one first record, no gap.
fn versions(
    db: &mut dyn Db,
    outcomes: usize,
    text: &CanonicalText,
    appraisal_id: i64,
) -> Result<Vec<StoredVersion>, StoreError> {
    let rows = fetch_all(
        db,
        "SELECT id, number, algorithm_version, ladder_version, created_at, frozen_at, closed_at
         FROM evidence_appraisal_versions WHERE appraisal_id = ? ORDER BY number",
        &[Value::from(appraisal_id)],
    )?;
    let mut versions = Vec::with_capacity(rows.len());
    for (index, r) in rows.iter().enumerate() {
        let id = r.get_i64("id")?;
        let bad = |why: String| StoreError::Corrupt(format!("appraisal version {id}: {why}"));
        let number = r.get_i64("number")?;
        let number = u32::try_from(number)
            .ok()
            .filter(|n| usize::try_from(*n).ok() == Some(index))
            .ok_or_else(|| {
                bad(format!(
                    "its number is {number}, not {index}: versions are 0, 1, 2… from one first record"
                ))
            })?;
        let algorithm_version = r.get_str("algorithm_version")?.to_string();
        let ladder_version = r.get_str("ladder_version")?.to_string();
        let earlier_rules =
            algorithm_version != ROB2_ALGORITHM_VERSION || ladder_version != LADDER_VERSION;
        let rows_of = |db: &mut dyn Db, table: &str, columns: &str, order: &str| {
            fetch_all(
                db,
                &format!("SELECT {columns} FROM {table} WHERE version_id = ? ORDER BY {order}"),
                &[Value::from(id)],
            )
        };
        let answers = rows_of(
            db,
            ANSWERS,
            ANSWER_COLUMNS,
            "domain, IFNULL(outcome, -1), question_id",
        )?
        .iter()
        .map(|r| answer_from(r, outcomes, !earlier_rules, text))
        .collect::<Result<_, _>>()
        .map_err(bad)?;
        let domains = rows_of(db, DOMAINS, DOMAIN_COLUMNS, "domain, IFNULL(outcome, -1)")?
            .iter()
            .map(|r| domain_from(r, outcomes))
            .collect::<Result<_, _>>()
            .map_err(bad)?;
        let overalls = rows_of(db, OVERALLS, OVERALL_COLUMNS, "outcome")?
            .iter()
            .map(|r| overall_from(r, outcomes))
            .collect::<Result<_, _>>()
            .map_err(bad)?;
        let opt = |name: &str| optional(r, name, text_of).map_err(bad);
        versions.push(StoredVersion {
            id,
            number,
            created_at: r.get_str("created_at")?.to_string(),
            frozen_at: opt("frozen_at")?,
            closed_at: opt("closed_at")?,
            algorithm_version,
            ladder_version,
            earlier_rules,
            answers,
            domains,
            overalls,
        });
    }
    Ok(versions)
}

/// Refuses an appraisal's `versions` unless the first record exists, is
/// frozen exactly when the appraisal is `saved` and never closed, revisions
/// come only after the save and are never frozen, and only the last may be
/// open.
fn check_versions(versions: &[StoredVersion], saved: bool) -> Result<(), String> {
    let Some((first, revisions)) = versions.split_first() else {
        return Err("it has no first record".into());
    };
    if first.frozen_at.is_some() != saved {
        return Err(format!(
            "its first record is {}frozen, but it is {}saved",
            if saved { "not " } else { "" },
            if saved { "" } else { "not " }
        ));
    }
    if first.closed_at.is_some() {
        return Err("its first record is closed; only a revision is".into());
    }
    if !saved && !revisions.is_empty() {
        return Err("it has a revision before its save".into());
    }
    if let Some(v) = revisions.iter().find(|v| v.frozen_at.is_some()) {
        return Err(format!(
            "revision {} is frozen; only the first record is",
            v.number
        ));
    }
    let before_last = &versions[..versions.len() - 1];
    if let Some(v) = before_last
        .iter()
        .find(|v| v.frozen_at.is_none() && v.closed_at.is_none())
    {
        return Err(format!(
            "version {} is open, but a later one exists",
            v.number
        ));
    }
    Ok(())
}

/// The domain and outcome of row `r`, checked against `outcomes` confirmed
/// outcomes ([`check_scope`]).
fn key_from(r: &Row, outcomes: usize) -> Result<(Domain, Option<usize>), String> {
    let name = r.get_str("domain").map_err(|e| e.to_string())?;
    let domain = domain_of(name).ok_or_else(|| format!("unknown domain {name:?}"))?;
    let outcome = optional(r, "outcome", Value::as_i64)?
        .map(|o| usize::try_from(o).map_err(|_| format!("outcome {o} is not an index")))
        .transpose()?;
    check_scope(domain, outcome, outcomes)?;
    Ok((domain, outcome))
}

/// The judgement stored in column `name` of `r`, if any.
fn judgement_of(r: &Row, name: &str) -> Result<Option<Judgement>, String> {
    optional(r, name, text_of)?
        .map(|j| from_wire(&j))
        .transpose()
}

/// Answer row `r` read back: its scope checked, its question one of
/// `questions(domain)` on a version under the `current` rules (any non-empty
/// id on one under earlier rules), and its mark inside `text`.
fn answer_from(
    r: &Row,
    outcomes: usize,
    current: bool,
    text: &CanonicalText,
) -> Result<ReaderAnswer, String> {
    let (domain, outcome) = key_from(r, outcomes)?;
    let question_id = r.get_str("question_id").map_err(|e| e.to_string())?;
    if current {
        check_question(domain, outcome, question_id, outcomes)?;
    } else if question_id.is_empty() {
        return Err("an answer names no question".into());
    }
    let int = |name: &str| optional(r, name, Value::as_i64);
    let mark = match (int("mark_start")?, int("mark_end")?) {
        (None, None) => None,
        (Some(start), Some(end)) => {
            let at = |v: i64| usize::try_from(v).map_err(|_| format!("a mark at {v}"));
            let span = TextSpan {
                start: at(start)?,
                end: at(end)?,
            };
            check_span(text, span)?;
            Some(span)
        }
        _ => return Err(format!("the mark of {question_id} is half stored")),
    };
    let tick = |name: &str| r.get_i64(name).map(|v| v != 0).map_err(|e| e.to_string());
    Ok(ReaderAnswer {
        domain,
        outcome,
        question_id: question_id.to_string(),
        answer: optional(r, "answer", text_of)?
            .map(|a| from_wire(&a))
            .transpose()?,
        mark,
        protocol: tick("used_protocol")?,
        registry: tick("used_registry")?,
        supplement: tick("used_supplement")?,
    })
}

/// Domain row `r` read back, its scope checked.
fn domain_from(r: &Row, outcomes: usize) -> Result<ReaderDomain, String> {
    let (domain, outcome) = key_from(r, outcomes)?;
    Ok(ReaderDomain {
        domain,
        outcome,
        derived: judgement_of(r, "derived")?,
        override_: judgement_of(r, "override")?,
        note: optional(r, "note", text_of)?,
    })
}

/// Overall row `r` read back, its outcome one of the `outcomes` confirmed.
fn overall_from(r: &Row, outcomes: usize) -> Result<ReaderOverall, String> {
    let outcome = r.get_i64("outcome").map_err(|e| e.to_string())?;
    let outcome =
        usize::try_from(outcome).map_err(|_| format!("outcome {outcome} is not an index"))?;
    check_outcome(outcome, outcomes)?;
    let text = |name: &str| optional(r, name, text_of);
    Ok(ReaderOverall {
        outcome,
        escalate: optional(r, "escalate", Value::as_i64)?.map(|e| e != 0),
        derived: judgement_of(r, "derived")?,
        override_: judgement_of(r, "override")?,
        ladder_start: text("ladder_start")?,
        ladder_level: text("ladder_level")?,
        ladder_weight: optional(r, "ladder_weight", Value::as_f64)?,
        ladder_error: text("ladder_error")?,
        ladder_steps_json: text("ladder_steps_json")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::APPRAISAL_NOTE_MAX_CHARS;
    use crate::evidence_store::tests::{kind_and_status, proposals_only, save_step};
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

    /// Pairs a new run on `cid` with appraisal `aid` and leaves it waiting
    /// with its design and two outcomes proposed.
    fn paired_run(c: &mut Connection, p: &PaperId, cid: i64, aid: i64) -> i64 {
        let run = create_run(c, p, cid, "m").unwrap();
        execute(
            c,
            "UPDATE evidence_runs SET appraisal_id = ? WHERE id = ?",
            &[Value::from(aid), Value::from(run)],
        )
        .unwrap();
        proposals_only(c, p, cid, run);
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
        let run = create_run(&mut c, &p, cid, "m").unwrap();
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
        let run = create_run(&mut c, &p, cid, "m").unwrap();
        execute(
            &mut c,
            "UPDATE evidence_runs SET appraisal_id = ? WHERE id = ?",
            &[Value::from(a), Value::from(run)],
        )
        .unwrap();
        refused(confirm(&mut c, a, &input(2)));
        let design = pending_steps(&mut c, run).unwrap().remove(0);
        save_step(&mut c, &p, cid, &design);
        refused(confirm(&mut c, a, &input(2)));
        proposals_only(&mut c, &p, cid, run);
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

    /// Saving freezes the first record and reveals; the next edit opens
    /// *after reveal 1*, copied from it, and later edits stay there.
    #[test]
    fn an_edit_after_save_opens_after_reveal_1_copied_from_0() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
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
        assert_eq!(answered(v0), vec![("1.1".into(), Some(Answer::Y))]);
        assert_eq!(v1.number, 1);
        assert!(v1.frozen_at.is_none() && v1.closed_at.is_none());
        assert_eq!(
            answered(v1),
            vec![
                ("1.1".into(), Some(Answer::Y)),
                ("1.2".into(), Some(Answer::N)),
                ("1.3".into(), Some(Answer::Py)),
            ]
        );
    }

    /// *Done revising* closes the open revision; the next edit opens *after
    /// reveal 2*, copied from 1. With no revision open, closing is refused.
    #[test]
    fn close_revision_then_edit_opens_after_reveal_2() {
        let mut c = open_memory_store().unwrap();
        let (_, a, _) = confirmed(&mut c);
        edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
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
        assert_eq!(
            answered(v1),
            vec![
                ("1.1".into(), Some(Answer::Y)),
                ("1.2".into(), Some(Answer::N))
            ]
        );
        assert_eq!(got.versions[2].number, 2);
        assert_eq!(
            answered(&got.versions[2]),
            vec![
                ("1.1".into(), Some(Answer::Pn)),
                ("1.2".into(), Some(Answer::N))
            ]
        );
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
        edit(&mut c, a, &answer(Domain::D1, None, "1.1", Answer::Y)).unwrap();
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
    /// key refuses outright), a question RoB 2 does not have, D1 naming an
    /// outcome, D2 naming one not confirmed, and a mark past the text's end.
    #[test]
    fn a_damaged_row_is_corrupt() {
        let damages = [
            "UPDATE evidence_appraisal_versions SET number = 1",
            "UPDATE evidence_appraisal_answers SET question_id = '1.9'",
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
}
