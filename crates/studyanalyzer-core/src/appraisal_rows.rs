//! The reader's appraisal read back (design 2026-11-26, §8): its rows as
//! types, and the shape check that reads a row this code did not write as
//! [`StoreError::Corrupt`]. The writers are in [`crate::appraisal_store`].

use bmlib::db::{fetch_all, fetch_one, Db, Row, Value};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::quality::{
    questions, study_design_from_str, Answer, Domain, Judgement, RctLayout, ReportedResult,
    StudyDesign, LADDER_VERSION, ROB2_ALGORITHM_VERSION,
};
use serde::Serialize;

use crate::appraisal::complete;
use crate::appraisal_store::rater_record;
use crate::evidence_rows::from_wire;
use crate::evidence_store::{covered, domain_name, domain_of};
use crate::pico_store::{optional, text_of};
use crate::store::{canonical_by_id, PaperId, StoreError};

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
pub(crate) const ANSWERS: &str = "evidence_appraisal_answers";
/// The table of a version's domain rows.
pub(crate) const DOMAINS: &str = "evidence_appraisal_domains";
/// The table of a version's overall rows.
pub(crate) const OVERALLS: &str = "evidence_appraisal_overalls";

/// An answer row's columns but its id and version, as copied and read.
pub(crate) const ANSWER_COLUMNS: &str =
    "domain, outcome, question_id, answer, mark_start, mark_end, \
    used_protocol, used_registry, used_supplement";
/// A domain row's columns but its id and version.
pub(crate) const DOMAIN_COLUMNS: &str = "domain, outcome, derived, override, note";
/// An overall row's columns but its id and version.
pub(crate) const OVERALL_COLUMNS: &str =
    "outcome, escalate, derived, override, ladder_start, ladder_level, \
    ladder_weight, ladder_error, ladder_steps_json";

/// The stored appraisal `appraisal_id`, or `None` if there is none: its row,
/// confirmed outcomes by position, and versions by number with their rows.
/// A row this code did not write is [`StoreError::Corrupt`]: a missing text,
/// a confirmation half stored or on a design bmlib does not know, versions
/// that are not 0, 1, 2… or with an open one before the last, a first record
/// frozen without a save (or the reverse), a first record frozen under the
/// current rules but not complete ([`complete`]), an answer naming no
/// question, D1 naming an outcome or D2–D5 naming none or one not confirmed,
/// a mark outside the text, or a result `ReportedResult::validate` refuses.
/// A question id the pinned bmlib does not have is read back as stored, on
/// any version: a re-pin can leave one in a draft, which a save or a
/// revision then carries on; it is never counted ([`rater_record`]).
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
    if let Some(v) = versions.iter().find(|v| {
        v.frozen_at.is_some() && !v.earlier_rules && !complete(&rater_record(v), outcomes.len())
    }) {
        return Err(bad(format!(
            "version {} is frozen, but not complete",
            v.number
        )));
    }
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

/// Whether `design` with `layout` is one RoB 2's parallel-group questions
/// cover ([`covered`]). A design or layout bmlib does not know, a randomised
/// trial without a layout, or a layout on any other design is an error.
pub(crate) fn design_covered(design: &str, layout: Option<&str>) -> Result<bool, String> {
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
    Ok(covered(d, layout))
}

/// Refuses `outcome` unless it is one of the `outcomes` confirmed.
pub(crate) fn check_outcome(outcome: usize, outcomes: usize) -> Result<(), String> {
    if outcome < outcomes {
        return Ok(());
    }
    Err(format!(
        "outcome {outcome} is not one of the {outcomes} confirmed"
    ))
}

/// Refuses a row of `domain` for `outcome` unless D1 names no outcome and
/// D2–D5 one of the `outcomes` confirmed.
pub(crate) fn check_scope(
    domain: Domain,
    outcome: Option<usize>,
    outcomes: usize,
) -> Result<(), String> {
    match (domain, outcome) {
        (Domain::D1, None) => Ok(()),
        (Domain::D1, Some(o)) => Err(format!("D1 is judged for the study, not for outcome {o}")),
        (_, Some(o)) => check_outcome(o, outcomes),
        (d, None) => Err(format!("{} is judged per outcome", domain_name(d))),
    }
}

/// Refuses question `question_id` of `domain` for `outcome` unless its scope
/// holds ([`check_scope`]) and it is one of `questions(domain)`.
pub(crate) fn check_question(
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
pub(crate) fn check_span(text: &CanonicalText, span: TextSpan) -> Result<(), String> {
    if span.start < span.end && text.span_text(span).is_some() {
        return Ok(());
    }
    Err(format!("the span {span} is not inside the paper's text"))
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
        .map(|r| answer_from(r, outcomes, text))
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

/// Answer row `r` read back: its scope checked, its question named (any
/// id: one the pinned bmlib does not have is kept, see [`appraisal`]), and
/// its mark inside `text`.
fn answer_from(r: &Row, outcomes: usize, text: &CanonicalText) -> Result<ReaderAnswer, String> {
    let (domain, outcome) = key_from(r, outcomes)?;
    let question_id = r.get_str("question_id").map_err(|e| e.to_string())?;
    if question_id.is_empty() {
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
