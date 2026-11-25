//! The results of an evidence run's steps as rows (design 2026-11-24, §6):
//! written from bmlib's results, read back as text, and turned back into
//! bmlib's types.
//!
//! **A run is shown as it was made.** The readers (`stored_*`) keep every
//! name bmlib gave as text and, on a run made under earlier rules, check
//! only the rows' shape, so such a run stays readable in full. On a run
//! under the current rules, question ids are held to bmlib's
//! `questions(domain)` (so `question_text` can name them), and designs,
//! layouts, scales, judgements, answers, D5 facts and verdicts, ladder
//! levels and reasons to the names today's bmlib writes; on an
//! earlier-rules run any non-empty one reads back. The escalation's own
//! three names are this app's and are checked on every run. Only the
//! conversions ([`proposed_outcome`], [`domain_result`], [`rob2_inputs`],
//! [`study_design`]) make bmlib's types, to hand stored results back to
//! bmlib on a run under the current rules; they refuse, as
//! [`StoreError::Corrupt`], what bmlib's own readers refuse.
//!
//! Citations are stored as spans only (`evidence_citations`): their wording
//! is re-read from the run's canonical text.

use std::collections::BTreeMap;

use bmlib::db::{execute, fetch_all, fetch_one, Db, Row, Value};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::quality::rob2_d5_checks::{D5Fact, FactVerdict};
use bmlib::quality::rob2_d5_facts::{AnalysisCheck, D5Facts, FactReport, OtherMeasurement};
use bmlib::quality::{
    questions, study_design_from_str, AssessedOutcome, DesignProposal, Domain, DomainAssessment,
    DomainResult, LadderReason, MachineAnswer, PicoCitation, PicoContradictedQuote,
    ProposedOutcome, ReportedResult, Rob2Escalation, Rob2Step, StudyDesign,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::evidence_store::{
    domain_name, domain_of, earlier_rules, step_by_id, step_from, EvidenceStep, StepStatus,
    StoredStep, STEP_COLUMNS, STEP_DESIGN, STEP_DOMAIN, STEP_OUTCOMES,
};
use crate::pico_store::{
    contradicted_to_json, contradiction_of, from_json, optional, text_of, to_json,
};
use crate::store::{last_id, StoreError};

// The names below are the ones bmlib writes (`the_spellings_are_bmlib_s`
// pins all but the ladder reasons, which `insert_outcome_result`'s match on
// `LadderReason` pins). The readers check a stored value against them, not
// against bmlib's types, and only on a run under the current rules (an
// earlier-rules run's may be any non-empty name).

/// Every study design's name (`StudyDesign::as_str`).
const DESIGNS: [&str; 15] = [
    "systematic_review",
    "meta_analysis",
    "rct",
    "cohort_prospective",
    "cohort_retrospective",
    "case_control",
    "cross_sectional",
    "case_series",
    "case_report",
    "guideline",
    "editorial",
    "letter",
    "comment",
    "other",
    "unknown",
];

/// Every randomised trial layout's name (`RctLayout::as_str`).
const RCT_LAYOUTS: [&str; 4] = ["parallel", "cluster", "crossover", "other"];

/// Every effect scale's name (`EffectScale`'s serde names).
const SCALES: [&str; 2] = ["ratio", "difference"];

/// Every RoB 2 judgement's name (`Judgement`'s serde names).
const JUDGEMENTS: [&str; 3] = ["low", "some_concerns", "high"];

/// Every RoB 2 answer's name (`Answer`'s serde names).
const ANSWERS: [&str; 5] = ["Y", "PY", "PN", "N", "NI"];

/// Every D5 fact's name (`D5Fact::as_str`).
const FACTS: [&str; 8] = [
    "plan_cited",
    "plan_dated_early",
    "plan_late",
    "measure_fixed",
    "timepoint_fixed",
    "analysis_fixed",
    "switched",
    "change_explained",
];

/// Every D5 fact verdict's name (`FactVerdict::as_str`).
const VERDICTS: [&str; 5] = [
    "present",
    "not_claimed",
    "no_quote_found",
    "quote_does_not_support",
    "no_plan_stated",
];

/// Every ladder level's name (`Level`'s serde names).
const LEVELS: [&str; 4] = ["high", "moderate", "low", "very_low"];

/// The ladder step taken for the risk of bias; it names the judgement.
const REASON_RISK_OF_BIAS: &str = "risk_of_bias";
/// The ladder step taken for an interval including no effect.
const REASON_CROSSES_NO_EFFECT: &str = "crosses_no_effect";
/// The ladder step taken for a small trial; it names `n` and the threshold.
const REASON_SMALL_TRIAL: &str = "small_trial";
/// The ladder step taken for no result or no interval.
const REASON_NO_INTERVAL: &str = "no_interval";
/// The ladder step shown when the number analysed is not reported.
const REASON_SAMPLE_SIZE_UNKNOWN: &str = "sample_size_unknown";
/// Every ladder step reason's name.
const REASONS: [&str; 5] = [
    REASON_RISK_OF_BIAS,
    REASON_CROSSES_NO_EFFECT,
    REASON_SMALL_TRIAL,
    REASON_NO_INTERVAL,
    REASON_SAMPLE_SIZE_UNKNOWN,
];

/// An escalation no domain called for; nothing was asked.
pub const ESCALATION_NOT_NEEDED: &str = "not_needed";
/// An escalation the model answered.
pub const ESCALATION_ASKED: &str = "asked";
/// An escalation asked and not answered.
pub const ESCALATION_FAILED: &str = "failed";

// The owners and roles of `evidence_citations` (migration 15's `CHECK`s).

/// A design's citations; the owner id is the design step's.
const OWNER_DESIGN: &str = "design";
/// An outcome's citations.
const OWNER_OUTCOME: &str = "outcome";
/// An answer's citations.
const OWNER_ANSWER: &str = "answer";
/// A D5 fact's citations.
const OWNER_FACT: &str = "fact";
/// A D5 other measurement's citations.
const OWNER_MEASUREMENT: &str = "measurement";
/// A D5 analysis check's citations; the owner id is the D5 step's.
const OWNER_ANALYSIS: &str = "analysis";
/// A quote backing a design, outcome, answer or fact.
const ROLE_QUOTE: &str = "quote";
/// A passage naming an other measurement.
const ROLE_NAMED: &str = "named";
/// A passage giving an other measurement's result.
const ROLE_RESULT: &str = "result";
/// What an other measurement's second look found.
const ROLE_SECOND_LOOK: &str = "second_look";
/// A passage naming the planned main analysis.
const ROLE_PLANNED: &str = "planned";
/// A passage giving the result with the analysis reported.
const ROLE_REPORTED: &str = "reported";

/// One stored citation: a span of the run's canonical text, its wording
/// re-read from there.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredCitation {
    /// The span's start, in code points.
    pub start: usize,
    /// The span's end, in code points.
    pub end: usize,
    /// How closely the model's quote matched the span.
    pub similarity: f64,
    /// Whether it matched exactly.
    pub exact: bool,
}

/// A quote the paper contradicts, with bmlib's name for why.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StoredContradicted {
    /// The model's quote, verbatim.
    pub quote: String,
    /// Why the paper contradicts it (`number`, `negation`, `direction`).
    pub contradiction: String,
}

/// The quotes backing one stored claim.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredQuotes {
    /// The spans found, in the order given.
    pub citations: Vec<StoredCitation>,
    /// The quotes not located in the paper, verbatim.
    pub not_found: Vec<String>,
    /// The quotes the paper contradicts.
    pub contradicted: Vec<StoredContradicted>,
    /// How many quote members were not usable text.
    pub dropped: usize,
}

/// A stored study design (`evidence_designs`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredDesign {
    /// The design step's id.
    pub step_id: i64,
    /// The design's name (`rct`, `cohort_prospective`, …).
    pub design: String,
    /// A randomised trial's layout (`parallel`, …).
    pub rct_layout: Option<String>,
    /// The model's explanation.
    pub reasoning: String,
    /// Its quotes.
    pub quotes: StoredQuotes,
}

/// An outcome's reported effect, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredResult {
    /// The measure's name, as the paper gives it.
    pub measure: String,
    /// `ratio` or `difference`.
    pub scale: String,
    /// The point estimate.
    pub estimate: f64,
    /// The interval's lower bound.
    pub ci_low: Option<f64>,
    /// The interval's upper bound.
    pub ci_high: Option<f64>,
    /// The interval's level.
    pub ci_level: Option<f64>,
    /// How many were analysed.
    pub n_analysed: Option<u32>,
}

/// A stored primary outcome (`evidence_outcomes`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredOutcome {
    /// The outcome's row id.
    pub id: i64,
    /// Its index among the run's outcomes, as RoB 2's steps name it.
    pub position: usize,
    /// The outcome as the paper names it.
    pub name: String,
    /// When it was measured.
    pub timepoint: Option<String>,
    /// Its reported effect.
    pub result: Option<StoredResult>,
    /// Why its reported effect was refused.
    pub result_error: Option<String>,
    /// Its quotes.
    pub quotes: StoredQuotes,
}

/// One answer to a signalling question (`evidence_answers`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredAnswer {
    /// The question's id (`2.3`), one of its domain's.
    pub question_id: String,
    /// The answer (`Y`, `PY`, `PN`, `N`, `NI`).
    pub answer: String,
    /// The model's explanation (D5's: the facts it was derived from).
    pub reasoning: String,
    /// Its quotes.
    pub quotes: StoredQuotes,
}

/// One D5 fact (`evidence_d5_facts`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredFact {
    /// The fact's name (`plan_cited`, …).
    pub fact: String,
    /// Whether the model said it is present.
    pub claimed: bool,
    /// Whether it counts, and if not, why (`present`, …).
    pub verdict: String,
    /// Its quotes.
    pub quotes: StoredQuotes,
}

/// One other measurement of D5's outcome (`evidence_d5_measurements`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredMeasurement {
    /// The model's short name for it.
    pub label: String,
    /// The passages naming it.
    pub named: Vec<StoredCitation>,
    /// The passages giving its result.
    pub result: Vec<StoredCitation>,
    /// What a second look found; `None` when none was made.
    pub second_look: Option<Vec<StoredCitation>>,
    /// Why the second look failed, when it did.
    pub second_look_error: Option<String>,
    /// Whether that request may succeed if sent again.
    pub second_look_retryable: bool,
    /// The HTTP status it was answered with.
    pub second_look_status: Option<u16>,
    /// Its quotes not located in the paper.
    pub not_found: Vec<String>,
    /// Its quotes the paper contradicts.
    pub contradicted: Vec<StoredContradicted>,
    /// How many of its quote members were not usable text.
    pub dropped: usize,
}

/// D5's check of the analysis reported against the one planned
/// (`evidence_d5_analyses`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredAnalysis {
    /// Whether the model says they differ with no reason given.
    pub differs: bool,
    /// The passages naming the planned main analysis.
    pub planned: Vec<StoredCitation>,
    /// The passages giving the result with its analysis.
    pub reported: Vec<StoredCitation>,
    /// Its quotes not located in the paper.
    pub not_found: Vec<String>,
    /// Its quotes the paper contradicts.
    pub contradicted: Vec<StoredContradicted>,
    /// How many of its quote members were not usable text.
    pub dropped: usize,
}

/// Everything D5 was answered from.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredD5 {
    /// The outcome's name, as the checks read it.
    pub outcome: String,
    /// Every fact, by name.
    pub facts: Vec<StoredFact>,
    /// The other measurements, in the reply's order.
    pub measurements: Vec<StoredMeasurement>,
    /// The analysis check.
    pub analysis: StoredAnalysis,
}

/// A stored, assessed domain (`evidence_domains`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredDomain {
    /// The domain step's id.
    pub step_id: i64,
    /// The domain (`D1` … `D5`).
    pub domain: String,
    /// Its judgement (`low`, `some_concerns`, `high`).
    pub judgement: String,
    /// Question ids answered although they did not apply.
    pub ignored: Vec<String>,
    /// The answers, by question id.
    pub answers: Vec<StoredAnswer>,
    /// D5's facts; `None` for every other domain.
    pub d5: Option<StoredD5>,
}

/// One step down an outcome's ladder (`evidence_ladder_steps`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredLadderStep {
    /// Why (`risk_of_bias`, `crosses_no_effect`, `small_trial`, …).
    pub reason: String,
    /// The overall judgement, for `risk_of_bias`.
    pub judgement: Option<String>,
    /// How many were analysed, for `small_trial`.
    pub n: Option<u32>,
    /// The threshold it fell under, for `small_trial`.
    pub threshold: Option<u32>,
    /// Levels lost.
    pub levels: u8,
}

/// An outcome's ladder, as built.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredLadder {
    /// Where it started (`high`, …).
    pub start: String,
    /// Where it ended.
    pub level: String,
    /// The end level's weight.
    pub weight: f64,
    /// Every step, risk of bias first.
    pub steps: Vec<StoredLadderStep>,
}

/// An outcome's escalation, overall judgement and ladder, stored with its
/// escalation step (`evidence_outcome_results`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredOutcomeResult {
    /// The escalation step's id.
    pub step_id: i64,
    /// [`ESCALATION_NOT_NEEDED`], [`ESCALATION_ASKED`] or
    /// [`ESCALATION_FAILED`] (the failure is the step's).
    pub escalation: String,
    /// The model's answer, when asked.
    pub escalate: Option<bool>,
    /// The model's explanation, when asked.
    pub escalation_reasoning: Option<String>,
    /// The overall judgement; `None` when not assessed.
    pub overall: Option<String>,
    /// The ladder, when one was built.
    pub ladder: Option<StoredLadder>,
    /// Why there is no ladder.
    pub ladder_error: Option<String>,
}

// --- Writing ---

/// The name bmlib's serde gives `value`, such as a `Judgement`'s.
fn wire<T: Serialize>(value: &T) -> Result<String, StoreError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => Ok(name),
        other => Err(StoreError::Corrupt(format!(
            "{other:?} is not a name that can be stored"
        ))),
    }
}

/// The three quote columns' values for `not_found`, `contradicted` and
/// `dropped`.
fn lost_values(
    not_found: &[String],
    contradicted: &[PicoContradictedQuote],
    dropped: usize,
) -> Result<[Value; 3], StoreError> {
    Ok([
        Value::from(to_json(not_found)?),
        Value::from(contradicted_to_json(contradicted)),
        Value::from(dropped),
    ])
}

/// The quote columns, in [`lost_values`]' order.
const LOST_COLUMNS: &str = "quotes_not_found_json, quotes_contradicted_json, quotes_dropped";

/// Stores `citations` of `owner` `owner_id` in `role`, each checked against
/// the run's `text`: one outside it is [`StoreError::Corrupt`].
fn insert_citations(
    db: &mut dyn Db,
    text: &CanonicalText,
    step_id: i64,
    owner: (&str, i64),
    role: &str,
    citations: &[PicoCitation],
) -> Result<(), StoreError> {
    for (position, c) in citations.iter().enumerate() {
        let span = TextSpan {
            start: c.start,
            end: c.end,
        };
        if text.span_text(span).is_none() {
            return Err(StoreError::Corrupt(format!(
                "citation {span} is outside the run's text"
            )));
        }
        execute(
            db,
            "INSERT INTO evidence_citations (step_id, owner, owner_id, role, position, span_start,
                                             span_end, similarity, exact)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            &[
                Value::from(step_id),
                Value::from(owner.0),
                Value::from(owner.1),
                Value::from(role),
                Value::from(position),
                Value::from(c.start),
                Value::from(c.end),
                Value::from(c.similarity),
                Value::from(c.exact),
            ],
        )?;
    }
    Ok(())
}

/// Inserts a row into `table` with `columns` then the quote columns, and
/// returns its id.
fn insert_with_lost(
    db: &mut dyn Db,
    table: &str,
    columns: &str,
    mut values: Vec<Value>,
    lost: [Value; 3],
) -> Result<i64, StoreError> {
    values.extend(lost);
    let marks = vec!["?"; values.len()].join(", ");
    execute(
        db,
        &format!("INSERT INTO {table} ({columns}, {LOST_COLUMNS}) VALUES ({marks})"),
        &values,
    )?;
    last_id(db)
}

/// Stores design step `step_id`'s `proposal` and its citations.
pub(crate) fn insert_design(
    db: &mut dyn Db,
    text: &CanonicalText,
    step_id: i64,
    p: &DesignProposal,
) -> Result<(), StoreError> {
    insert_with_lost(
        db,
        "evidence_designs",
        "step_id, design, rct_layout, reasoning",
        vec![
            Value::from(step_id),
            Value::from(p.design.as_str()),
            Value::from(p.rct_layout.map(|l| l.as_str())),
            Value::from(p.reasoning.as_str()),
        ],
        lost_values(&p.quotes_not_found, &p.contradicted, p.quotes_dropped)?,
    )?;
    let owner = (OWNER_DESIGN, step_id);
    insert_citations(db, text, step_id, owner, ROLE_QUOTE, &p.citations)
}

/// Stores outcome `position` of outcomes step `step_id` and its citations.
pub(crate) fn insert_outcome(
    db: &mut dyn Db,
    text: &CanonicalText,
    step_id: i64,
    position: usize,
    o: &ProposedOutcome,
) -> Result<(), StoreError> {
    let r = o.result.as_ref();
    let id = insert_with_lost(
        db,
        "evidence_outcomes",
        "step_id, position, name, timepoint, measure, scale, estimate, ci_low, ci_high, \
         ci_level, n_analysed, result_error",
        vec![
            Value::from(step_id),
            Value::from(position),
            Value::from(o.name.as_str()),
            Value::from(o.timepoint.as_deref()),
            Value::from(r.map(|r| r.measure.as_str())),
            Value::from(r.map(|r| wire(&r.scale)).transpose()?),
            Value::from(r.map(|r| r.estimate)),
            Value::from(r.and_then(|r| r.ci_low)),
            Value::from(r.and_then(|r| r.ci_high)),
            Value::from(r.and_then(|r| r.ci_level)),
            Value::from(r.and_then(|r| r.n_analysed).map(i64::from)),
            Value::from(o.result_error.as_deref()),
        ],
        lost_values(&o.quotes_not_found, &o.contradicted, o.quotes_dropped)?,
    )?;
    insert_citations(
        db,
        text,
        step_id,
        (OWNER_OUTCOME, id),
        ROLE_QUOTE,
        &o.citations,
    )
}

/// Stores domain step `step_id`'s assessment `a`: its judgement, answers and
/// citations, and D5's facts, measurements and analysis check. Facts on a
/// domain that is not D5, or D5 without them, is [`StoreError::Corrupt`].
pub(crate) fn insert_domain(
    db: &mut dyn Db,
    text: &CanonicalText,
    step_id: i64,
    a: &DomainAssessment,
) -> Result<(), StoreError> {
    if a.facts.is_some() != (a.domain == Domain::D5) {
        return Err(StoreError::Corrupt(format!(
            "evidence step {step_id}: facts are D5's, and D5 needs them; this is {}",
            domain_name(a.domain)
        )));
    }
    execute(
        db,
        "INSERT INTO evidence_domains (step_id, judgement, ignored_json, facts_outcome)
         VALUES (?, ?, ?, ?)",
        &[
            Value::from(step_id),
            Value::from(wire(&a.judgement)?),
            Value::from(to_json(&a.ignored)?),
            Value::from(a.facts.as_ref().map(|f| f.outcome.as_str())),
        ],
    )?;
    for (question, answer) in &a.answers {
        let id = insert_with_lost(
            db,
            "evidence_answers",
            "step_id, question_id, answer, reasoning",
            vec![
                Value::from(step_id),
                Value::from(question.as_str()),
                Value::from(wire(&answer.answer)?),
                Value::from(answer.reasoning.as_str()),
            ],
            lost_values(
                &answer.quotes_not_found,
                &answer.contradicted,
                answer.quotes_dropped,
            )?,
        )?;
        let owner = (OWNER_ANSWER, id);
        insert_citations(db, text, step_id, owner, ROLE_QUOTE, &answer.citations)?;
    }
    let Some(facts) = &a.facts else {
        return Ok(());
    };
    for (fact, report) in &facts.facts {
        let id = insert_with_lost(
            db,
            "evidence_d5_facts",
            "step_id, fact, claimed, verdict",
            vec![
                Value::from(step_id),
                Value::from(fact.as_str()),
                Value::from(report.claimed),
                Value::from(report.verdict.as_str()),
            ],
            lost_values(
                &report.quotes_not_found,
                &report.contradicted,
                report.quotes_dropped,
            )?,
        )?;
        let owner = (OWNER_FACT, id);
        insert_citations(db, text, step_id, owner, ROLE_QUOTE, &report.citations)?;
    }
    for (position, m) in facts.measurements.iter().enumerate() {
        let id = insert_with_lost(
            db,
            "evidence_d5_measurements",
            "step_id, position, label, second_look_asked, second_look_error, \
             second_look_retryable, second_look_status",
            vec![
                Value::from(step_id),
                Value::from(position),
                Value::from(m.label.as_str()),
                Value::from(m.second_look.is_some()),
                Value::from(m.second_look_error.as_deref()),
                Value::from(m.second_look_retryable),
                Value::from(m.second_look_status.map(i64::from)),
            ],
            lost_values(&m.quotes_not_found, &m.contradicted, m.quotes_dropped)?,
        )?;
        let owner = (OWNER_MEASUREMENT, id);
        insert_citations(db, text, step_id, owner, ROLE_NAMED, &m.named)?;
        insert_citations(db, text, step_id, owner, ROLE_RESULT, &m.result)?;
        let look = m.second_look.as_deref().unwrap_or_default();
        insert_citations(db, text, step_id, owner, ROLE_SECOND_LOOK, look)?;
    }
    let check = &facts.analysis;
    insert_with_lost(
        db,
        "evidence_d5_analyses",
        "step_id, differs",
        vec![Value::from(step_id), Value::from(check.differs)],
        lost_values(
            &check.quotes_not_found,
            &check.contradicted,
            check.quotes_dropped,
        )?,
    )?;
    let owner = (OWNER_ANALYSIS, step_id);
    insert_citations(db, text, step_id, owner, ROLE_PLANNED, &check.planned)?;
    insert_citations(db, text, step_id, owner, ROLE_REPORTED, &check.reported)
}

/// Stores escalation step `step_id`'s `escalation` and its outcome's overall
/// judgement and ladder (or ladder error) from `assessed`.
pub(crate) fn insert_outcome_result(
    db: &mut dyn Db,
    step_id: i64,
    escalation: &Rob2Escalation,
    assessed: &AssessedOutcome,
) -> Result<(), StoreError> {
    let (kind, escalate, reasoning) = match escalation {
        Rob2Escalation::NotNeeded => (ESCALATION_NOT_NEEDED, None, None),
        Rob2Escalation::Asked(e) => (
            ESCALATION_ASKED,
            Some(e.escalate),
            Some(e.reasoning.as_str()),
        ),
        Rob2Escalation::Failed { .. } => (ESCALATION_FAILED, None, None),
    };
    let ladder = assessed.ladder.as_ref();
    execute(
        db,
        "INSERT INTO evidence_outcome_results (step_id, escalation, escalate, escalation_reasoning,
                                               overall, ladder_start, ladder_level, ladder_weight,
                                               ladder_error)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            Value::from(step_id),
            Value::from(kind),
            Value::from(escalate),
            Value::from(reasoning),
            Value::from(assessed.rob2.overall.map(|j| wire(&j)).transpose()?),
            Value::from(ladder.map(|l| wire(&l.start)).transpose()?),
            Value::from(ladder.map(|l| wire(&l.level)).transpose()?),
            Value::from(ladder.map(|l| l.weight)),
            Value::from(assessed.ladder_error.as_deref()),
        ],
    )?;
    for (position, s) in ladder
        .map(|l| l.steps.as_slice())
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let (reason, judgement, n, threshold) = match s.reason {
            LadderReason::RiskOfBias(j) => (REASON_RISK_OF_BIAS, Some(wire(&j)?), None, None),
            LadderReason::CrossesNoEffect => (REASON_CROSSES_NO_EFFECT, None, None, None),
            LadderReason::SmallTrial { n, threshold } => {
                (REASON_SMALL_TRIAL, None, Some(n), Some(threshold))
            }
            LadderReason::NoInterval => (REASON_NO_INTERVAL, None, None, None),
            LadderReason::SampleSizeUnknown => (REASON_SAMPLE_SIZE_UNKNOWN, None, None, None),
        };
        execute(
            db,
            "INSERT INTO evidence_ladder_steps (step_id, position, reason, judgement, n,
                                                threshold, levels)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            &[
                Value::from(step_id),
                Value::from(position),
                Value::from(reason),
                Value::from(judgement),
                Value::from(n.map(i64::from)),
                Value::from(threshold.map(i64::from)),
                Value::from(i64::from(s.levels)),
            ],
        )?;
    }
    Ok(())
}

/// Every table that holds a step's result, keyed by its `step_id`.
pub(crate) const RESULT_TABLES: [&str; 10] = [
    "evidence_citations",
    "evidence_designs",
    "evidence_outcomes",
    "evidence_domains",
    "evidence_answers",
    "evidence_d5_facts",
    "evidence_d5_measurements",
    "evidence_d5_analyses",
    "evidence_outcome_results",
    "evidence_ladder_steps",
];

/// Deletes step `step_id`'s stored result, whatever its kind.
pub(crate) fn delete_results(db: &mut dyn Db, step_id: i64) -> Result<(), StoreError> {
    for table in RESULT_TABLES {
        execute(
            db,
            &format!("DELETE FROM {table} WHERE step_id = ?"),
            &[Value::from(step_id)],
        )?;
    }
    Ok(())
}

// --- Reading, as text ---

/// `Corrupt`, naming evidence step `step_id`.
fn corrupt(step_id: i64, why: impl std::fmt::Display) -> StoreError {
    StoreError::Corrupt(format!("evidence step {step_id}: {why}"))
}

/// Refuses `value` in column `column` unless it is one of `known`.
fn known(step_id: i64, column: &str, value: &str, known: &[&str]) -> Result<(), StoreError> {
    if known.contains(&value) {
        return Ok(());
    }
    Err(corrupt(step_id, format!("unknown {column} {value:?}")))
}

/// [`known`] on a run under the current rules; on a run under `earlier`
/// rules, whose names may not be today's, only an empty `value` is refused.
fn known_unless_earlier(
    step_id: i64,
    column: &str,
    value: &str,
    names: &[&str],
    earlier: bool,
) -> Result<(), StoreError> {
    if earlier && !value.is_empty() {
        return Ok(());
    }
    known(step_id, column, value, names)
}

/// Refuses row `r` of step `step_id`'s `what`s unless it is stored at
/// `index`: rows read in position order must sit at 0, 1, 2….
fn check_position(step_id: i64, what: &str, r: &Row, index: usize) -> Result<(), StoreError> {
    let position = count_of(step_id, "position", r.get_i64("position")?)?;
    if position == index {
        return Ok(());
    }
    Err(corrupt(
        step_id,
        format!("{what} {index} is at position {position}"),
    ))
}

/// `count` as a count, refusing a negative one.
fn count_of(step_id: i64, column: &str, count: i64) -> Result<usize, StoreError> {
    usize::try_from(count).map_err(|_| corrupt(step_id, format!("a negative {column}")))
}

/// The citations of `owner` `owner_id` in `role`, in their order.
fn citations(
    db: &mut dyn Db,
    step_id: i64,
    owner: (&str, i64),
    role: &str,
) -> Result<Vec<StoredCitation>, StoreError> {
    fetch_all(
        db,
        "SELECT span_start, span_end, similarity, exact FROM evidence_citations
         WHERE step_id = ? AND owner = ? AND owner_id = ? AND role = ? ORDER BY position",
        &[
            Value::from(step_id),
            Value::from(owner.0),
            Value::from(owner.1),
            Value::from(role),
        ],
    )?
    .iter()
    .map(|r| {
        Ok(StoredCitation {
            start: count_of(step_id, "span", r.get_i64("span_start")?)?,
            end: count_of(step_id, "span", r.get_i64("span_end")?)?,
            similarity: r
                .get("similarity")?
                .as_f64()
                .ok_or_else(|| corrupt(step_id, "a similarity is not a number"))?,
            exact: r.get_i64("exact")? != 0,
        })
    })
    .collect()
}

/// A row's quote columns: the quotes not found, contradicted and dropped.
/// Each contradiction is a name bmlib writes, or, on a run under `earlier`
/// rules, any name but an empty one.
fn lost_from(
    step_id: i64,
    r: &Row,
    earlier: bool,
) -> Result<(Vec<String>, Vec<StoredContradicted>, usize), StoreError> {
    let contradicted: Vec<StoredContradicted> =
        serde_json::from_str(r.get_str("quotes_contradicted_json")?)
            .map_err(|e| corrupt(step_id, format!("its contradicted quotes are invalid: {e}")))?;
    for q in &contradicted {
        if !earlier || q.contradiction.is_empty() {
            contradiction_of(&q.contradiction).map_err(|e| corrupt(step_id, e))?;
        }
    }
    Ok((
        from_json(r.get_str("quotes_not_found_json")?).map_err(|e| corrupt(step_id, e))?,
        contradicted,
        count_of(step_id, "dropped count", r.get_i64("quotes_dropped")?)?,
    ))
}

/// A row's quotes: its `quote` citations and its quote columns.
fn quotes_from(
    db: &mut dyn Db,
    step_id: i64,
    owner: (&str, i64),
    r: &Row,
    earlier: bool,
) -> Result<StoredQuotes, StoreError> {
    let (not_found, contradicted, dropped) = lost_from(step_id, r, earlier)?;
    Ok(StoredQuotes {
        citations: citations(db, step_id, owner, ROLE_QUOTE)?,
        not_found,
        contradicted,
        dropped,
    })
}

/// Column `name` of `r` as optional text.
fn opt_text(step_id: i64, r: &Row, name: &str) -> Result<Option<String>, StoreError> {
    optional(r, name, text_of).map_err(|e| corrupt(step_id, e))
}

/// Column `name` of `r` as an optional whole number.
fn opt_int(step_id: i64, r: &Row, name: &str) -> Result<Option<i64>, StoreError> {
    optional(r, name, Value::as_i64).map_err(|e| corrupt(step_id, e))
}

/// Column `name` of `r` as an optional number.
fn opt_real(step_id: i64, r: &Row, name: &str) -> Result<Option<f64>, StoreError> {
    optional(r, name, Value::as_f64).map_err(|e| corrupt(step_id, e))
}

/// `value` of column `name`, which must fit 32 bits.
fn u32_of(step_id: i64, name: &str, value: Option<i64>) -> Result<Option<u32>, StoreError> {
    value
        .map(|v| {
            u32::try_from(v).map_err(|_| corrupt(step_id, format!("{name} {v} is out of range")))
        })
        .transpose()
}

/// The one step of `kind` of run `run_id` (its design or outcomes step), or
/// `None` when the run has none yet.
fn step_of_kind(
    db: &mut dyn Db,
    run_id: i64,
    kind: &str,
) -> Result<Option<StoredStep>, StoreError> {
    fetch_one(
        db,
        &format!("SELECT {STEP_COLUMNS} FROM evidence_steps WHERE run_id = ? AND kind = ?"),
        &[Value::from(run_id), Value::from(kind)],
    )?
    .map(|r| step_from(&r))
    .transpose()
}

/// Refuses a result row on `step` unless it holds one exactly when done (a
/// failed escalation may hold one too).
fn check_held(step: &StoredStep, held: bool) -> Result<(), StoreError> {
    let may_fail_with_one = matches!(step.step, EvidenceStep::Rob2(Rob2Step::Escalation { .. }))
        && step.status == StepStatus::Failed;
    match (step.status, held) {
        (StepStatus::Done, false) => {
            Err(corrupt(step.id, "it is done, but its result is not stored"))
        }
        (StepStatus::Pending | StepStatus::Failed, true) if !may_fail_with_one => Err(corrupt(
            step.id,
            format!("it is {}, but holds a result", step.status.as_str()),
        )),
        _ => Ok(()),
    }
}

/// The design run `run_id` stored, or `None` while its design step is not
/// done (or there is no such run). Its design and layout are checked to be
/// names bmlib writes (on an earlier-rules run, only not empty); a done step
/// without its row, or a row on a step not done, is [`StoreError::Corrupt`].
pub fn stored_design(db: &mut dyn Db, run_id: i64) -> Result<Option<StoredDesign>, StoreError> {
    let Some(step) = step_of_kind(db, run_id, STEP_DESIGN)? else {
        return Ok(None);
    };
    let row = fetch_one(
        db,
        &format!(
            "SELECT design, rct_layout, reasoning, {LOST_COLUMNS} FROM evidence_designs
             WHERE step_id = ?"
        ),
        &[Value::from(step.id)],
    )?;
    check_held(&step, row.is_some())?;
    let Some(r) = row else {
        return Ok(None);
    };
    let id = step.id;
    let earlier = earlier_rules(db, run_id)?;
    let design = r.get_str("design")?.to_string();
    known_unless_earlier(id, "design", &design, &DESIGNS, earlier)?;
    let rct_layout = opt_text(id, &r, "rct_layout")?;
    if let Some(layout) = &rct_layout {
        known_unless_earlier(id, "layout", layout, &RCT_LAYOUTS, earlier)?;
    }
    Ok(Some(StoredDesign {
        step_id: id,
        design,
        rct_layout,
        reasoning: r.get_str("reasoning")?.to_string(),
        quotes: quotes_from(db, id, (OWNER_DESIGN, id), &r, earlier)?,
    }))
}

/// The primary outcomes run `run_id` stored, in order (none while its
/// outcomes step is not done, or when the paper names none). A result is
/// stored whole or not at all, its scale a name bmlib writes (on an
/// earlier-rules run, only not empty); outcomes on a step not done, or
/// positions that are not 0, 1, 2…, are [`StoreError::Corrupt`].
pub fn stored_outcomes(db: &mut dyn Db, run_id: i64) -> Result<Vec<StoredOutcome>, StoreError> {
    let Some(step) = step_of_kind(db, run_id, STEP_OUTCOMES)? else {
        return Ok(Vec::new());
    };
    let id = step.id;
    let rows = fetch_all(
        db,
        &format!(
            "SELECT id, position, name, timepoint, measure, scale, estimate, ci_low, ci_high,
                    ci_level, n_analysed, result_error, {LOST_COLUMNS}
             FROM evidence_outcomes WHERE step_id = ? ORDER BY position"
        ),
        &[Value::from(id)],
    )?;
    if !rows.is_empty() {
        check_held(&step, true)?;
    } else if step.status != StepStatus::Done {
        return Ok(Vec::new());
    }
    let earlier = earlier_rules(db, run_id)?;
    let mut outcomes = Vec::with_capacity(rows.len());
    for (index, r) in rows.iter().enumerate() {
        check_position(id, "outcome", r, index)?;
        let result = match (
            opt_text(id, r, "measure")?,
            opt_text(id, r, "scale")?,
            opt_real(id, r, "estimate")?,
        ) {
            (None, None, None) => None,
            (Some(measure), Some(scale), Some(estimate)) => {
                known_unless_earlier(id, "scale", &scale, &SCALES, earlier)?;
                Some(StoredResult {
                    measure,
                    scale,
                    estimate,
                    ci_low: opt_real(id, r, "ci_low")?,
                    ci_high: opt_real(id, r, "ci_high")?,
                    ci_level: opt_real(id, r, "ci_level")?,
                    n_analysed: u32_of(id, "n_analysed", opt_int(id, r, "n_analysed")?)?,
                })
            }
            _ => {
                return Err(corrupt(
                    id,
                    format!("outcome {index}'s result is half stored"),
                ))
            }
        };
        let row_id = r.get_i64("id")?;
        outcomes.push(StoredOutcome {
            id: row_id,
            position: index,
            name: r.get_str("name")?.to_string(),
            timepoint: opt_text(id, r, "timepoint")?,
            result,
            result_error: opt_text(id, r, "result_error")?,
            quotes: quotes_from(db, id, (OWNER_OUTCOME, row_id), r, earlier)?,
        });
    }
    Ok(outcomes)
}

/// The domain `step` assesses; a step that is not a domain step is
/// [`StoreError::Corrupt`].
fn domain_of_step(step: &StoredStep) -> Result<Domain, StoreError> {
    match step.step {
        EvidenceStep::Rob2(Rob2Step::Domain { domain, .. }) => Ok(domain),
        other => Err(corrupt(step.id, format!("{other:?} is not a domain step"))),
    }
}

/// Refuses `question` unless it is one of `domain`'s, so bmlib's
/// `question_text` can name it; on a run under `earlier` rules, whose
/// questions may not be today's, only an empty id is refused.
fn check_question(
    step_id: i64,
    domain: Domain,
    question: &str,
    earlier: bool,
) -> Result<(), StoreError> {
    let fits = if earlier {
        !question.is_empty()
    } else {
        questions(domain).contains(&question)
    };
    if fits {
        return Ok(());
    }
    Err(corrupt(
        step_id,
        format!("{question:?} is not a question of {}", domain_name(domain)),
    ))
}

/// Domain step `step_id`'s stored assessment, or `None` while it is not done.
/// Answers come by question id, D5's facts by name and its measurements in
/// order. The shape is checked: a done step has its row and a step not done
/// none; facts, measurements and an analysis check exactly on D5; every
/// question id (answered or ignored) one of its domain's on a run under the
/// current rules (so `question_text` never panics), and not empty on an
/// earlier-rules run; every judgement, answer, fact and verdict a name bmlib
/// writes (on an earlier-rules run, only not empty).
/// Otherwise [`StoreError::Corrupt`]; no such step is
/// [`StoreError::NotFound`].
pub fn stored_domain(db: &mut dyn Db, step_id: i64) -> Result<Option<StoredDomain>, StoreError> {
    let (run_id, step) = step_by_id(db, step_id)?
        .ok_or_else(|| StoreError::NotFound(format!("evidence step {step_id}")))?;
    let earlier = earlier_rules(db, run_id)?;
    let domain = domain_of_step(&step)?;
    let row = fetch_one(
        db,
        "SELECT judgement, ignored_json, facts_outcome FROM evidence_domains WHERE step_id = ?",
        &[Value::from(step_id)],
    )?;
    check_held(&step, row.is_some())?;
    let Some(r) = row else {
        return Ok(None);
    };
    let id = step_id;
    let judgement = r.get_str("judgement")?.to_string();
    known_unless_earlier(id, "judgement", &judgement, &JUDGEMENTS, earlier)?;
    let ignored = from_json(r.get_str("ignored_json")?).map_err(|e| corrupt(id, e))?;
    for question in &ignored {
        check_question(id, domain, question, earlier)?;
    }
    let mut answers = Vec::new();
    let rows = fetch_all(
        db,
        &format!(
            "SELECT id, question_id, answer, reasoning, {LOST_COLUMNS} FROM evidence_answers
             WHERE step_id = ? ORDER BY question_id"
        ),
        &[Value::from(id)],
    )?;
    for a in &rows {
        let question_id = a.get_str("question_id")?.to_string();
        check_question(id, domain, &question_id, earlier)?;
        let answer = a.get_str("answer")?.to_string();
        known_unless_earlier(id, "answer", &answer, &ANSWERS, earlier)?;
        answers.push(StoredAnswer {
            question_id,
            answer,
            reasoning: a.get_str("reasoning")?.to_string(),
            quotes: quotes_from(db, id, (OWNER_ANSWER, a.get_i64("id")?), a, earlier)?,
        });
    }
    let facts_outcome = opt_text(id, &r, "facts_outcome")?;
    let d5 = match facts_outcome {
        Some(outcome) if domain == Domain::D5 => Some(stored_d5(db, id, outcome, earlier)?),
        None if domain != Domain::D5 => {
            check_no_facts(db, id)?;
            None
        }
        _ => return Err(corrupt(id, "facts are stored exactly for D5")),
    };
    Ok(Some(StoredDomain {
        step_id: id,
        domain: domain_name(domain).to_string(),
        judgement,
        ignored,
        answers,
        d5,
    }))
}

/// Refuses D5's rows on domain step `step_id`, which is not D5.
fn check_no_facts(db: &mut dyn Db, step_id: i64) -> Result<(), StoreError> {
    for table in [
        "evidence_d5_facts",
        "evidence_d5_measurements",
        "evidence_d5_analyses",
    ] {
        let held = fetch_one(
            db,
            &format!("SELECT 1 AS one FROM {table} WHERE step_id = ? LIMIT 1"),
            &[Value::from(step_id)],
        )?;
        if held.is_some() {
            return Err(corrupt(
                step_id,
                format!("{table} holds rows of a domain that is not D5"),
            ));
        }
    }
    Ok(())
}

/// D5 step `step_id`'s facts, other measurements and analysis check. Each
/// fact and verdict is a name bmlib writes, or, on a run under `earlier`
/// rules, any name but an empty one.
fn stored_d5(
    db: &mut dyn Db,
    step_id: i64,
    outcome: String,
    earlier: bool,
) -> Result<StoredD5, StoreError> {
    let id = step_id;
    let mut facts = Vec::new();
    let rows = fetch_all(
        db,
        &format!(
            "SELECT id, fact, claimed, verdict, {LOST_COLUMNS} FROM evidence_d5_facts
             WHERE step_id = ? ORDER BY fact"
        ),
        &[Value::from(id)],
    )?;
    for f in &rows {
        let fact = f.get_str("fact")?.to_string();
        known_unless_earlier(id, "fact", &fact, &FACTS, earlier)?;
        let verdict = f.get_str("verdict")?.to_string();
        known_unless_earlier(id, "verdict", &verdict, &VERDICTS, earlier)?;
        facts.push(StoredFact {
            fact,
            claimed: f.get_i64("claimed")? != 0,
            verdict,
            quotes: quotes_from(db, id, (OWNER_FACT, f.get_i64("id")?), f, earlier)?,
        });
    }
    let mut measurements = Vec::new();
    let rows = fetch_all(
        db,
        &format!(
            "SELECT id, position, label, second_look_asked, second_look_error,
                    second_look_retryable, second_look_status, {LOST_COLUMNS}
             FROM evidence_d5_measurements WHERE step_id = ? ORDER BY position"
        ),
        &[Value::from(id)],
    )?;
    for (index, m) in rows.iter().enumerate() {
        check_position(id, "measurement", m, index)?;
        let owner = (OWNER_MEASUREMENT, m.get_i64("id")?);
        let error = opt_text(id, m, "second_look_error")?;
        let retryable = m.get_i64("second_look_retryable")? != 0;
        let status = opt_int(id, m, "second_look_status")?
            .map(|s| {
                u16::try_from(s).map_err(|_| corrupt(id, format!("{s} is not an HTTP status")))
            })
            .transpose()?;
        if error.is_none() && (retryable || status.is_some()) {
            return Err(corrupt(id, "a second look's failure is half stored"));
        }
        let looked = citations(db, id, owner, ROLE_SECOND_LOOK)?;
        let second_look = if m.get_i64("second_look_asked")? != 0 {
            Some(looked)
        } else if looked.is_empty() {
            None
        } else {
            return Err(corrupt(id, "a second look never made holds passages"));
        };
        let (not_found, contradicted, dropped) = lost_from(id, m, earlier)?;
        measurements.push(StoredMeasurement {
            label: m.get_str("label")?.to_string(),
            named: citations(db, id, owner, ROLE_NAMED)?,
            result: citations(db, id, owner, ROLE_RESULT)?,
            second_look,
            second_look_error: error,
            second_look_retryable: retryable,
            second_look_status: status,
            not_found,
            contradicted,
            dropped,
        });
    }
    let a = fetch_one(
        db,
        &format!("SELECT differs, {LOST_COLUMNS} FROM evidence_d5_analyses WHERE step_id = ?"),
        &[Value::from(id)],
    )?
    .ok_or_else(|| corrupt(id, "D5's analysis check is not stored"))?;
    let owner = (OWNER_ANALYSIS, id);
    let (not_found, contradicted, dropped) = lost_from(id, &a, earlier)?;
    let analysis = StoredAnalysis {
        differs: a.get_i64("differs")? != 0,
        planned: citations(db, id, owner, ROLE_PLANNED)?,
        reported: citations(db, id, owner, ROLE_REPORTED)?,
        not_found,
        contradicted,
        dropped,
    };
    Ok(StoredD5 {
        outcome,
        facts,
        measurements,
        analysis,
    })
}

/// Escalation step `step_id`'s stored outcome result, or `None` while there
/// is none (pending, or failed by [`crate::evidence_store::fail_step`]). The
/// escalation as stored (`escalate` and its reasoning exactly when asked;
/// `failed` exactly on a failed step), the overall judgement, and the ladder
/// (steps in order) or its error, never both; every judgement, level and
/// reason a name bmlib writes (on an earlier-rules run, only not empty),
/// with `judgement`, `n` and `threshold` exactly where a reason today's
/// bmlib writes names them (any other reason is read as stored). Otherwise
/// [`StoreError::Corrupt`]; no such step is [`StoreError::NotFound`].
pub fn stored_outcome_result(
    db: &mut dyn Db,
    step_id: i64,
) -> Result<Option<StoredOutcomeResult>, StoreError> {
    let (run_id, step) = step_by_id(db, step_id)?
        .ok_or_else(|| StoreError::NotFound(format!("evidence step {step_id}")))?;
    if !matches!(step.step, EvidenceStep::Rob2(Rob2Step::Escalation { .. })) {
        return Err(corrupt(
            step_id,
            format!("{:?} is not an escalation", step.step),
        ));
    }
    let row = fetch_one(
        db,
        "SELECT escalation, escalate, escalation_reasoning, overall, ladder_start, ladder_level,
                ladder_weight, ladder_error
         FROM evidence_outcome_results WHERE step_id = ?",
        &[Value::from(step_id)],
    )?;
    check_held(&step, row.is_some())?;
    let Some(r) = row else {
        return Ok(None);
    };
    let id = step_id;
    let escalation = r.get_str("escalation")?.to_string();
    let escalate = opt_int(id, &r, "escalate")?.map(|e| e != 0);
    let reasoning = opt_text(id, &r, "escalation_reasoning")?;
    let asked = escalation == ESCALATION_ASKED;
    if (escalation == ESCALATION_FAILED) != (step.status == StepStatus::Failed)
        || escalate.is_some() != asked
        || reasoning.is_some() != asked
    {
        return Err(corrupt(
            id,
            format!(
                "a {} step's escalation {escalation:?} does not fit it",
                step.status.as_str()
            ),
        ));
    }
    known(
        id,
        "escalation",
        &escalation,
        &[ESCALATION_NOT_NEEDED, ESCALATION_ASKED, ESCALATION_FAILED],
    )?;
    let earlier = earlier_rules(db, run_id)?;
    let overall = opt_text(id, &r, "overall")?;
    if let Some(j) = &overall {
        known_unless_earlier(id, "judgement", j, &JUDGEMENTS, earlier)?;
    }
    let ladder = match (
        opt_text(id, &r, "ladder_start")?,
        opt_text(id, &r, "ladder_level")?,
        opt_real(id, &r, "ladder_weight")?,
    ) {
        (None, None, None) => None,
        (Some(start), Some(level), Some(weight)) if overall.is_some() => {
            known_unless_earlier(id, "level", &start, &LEVELS, earlier)?;
            known_unless_earlier(id, "level", &level, &LEVELS, earlier)?;
            Some(StoredLadder {
                start,
                level,
                weight,
                steps: ladder_steps(db, id, earlier)?,
            })
        }
        _ => {
            return Err(corrupt(
                id,
                "its ladder is half stored, or has no overall judgement",
            ))
        }
    };
    let ladder_error = opt_text(id, &r, "ladder_error")?;
    if ladder.is_some() == ladder_error.is_some() {
        return Err(corrupt(
            id,
            "it holds both a ladder and a ladder error, or neither",
        ));
    }
    Ok(Some(StoredOutcomeResult {
        step_id: id,
        escalation,
        escalate,
        escalation_reasoning: reasoning,
        overall,
        ladder,
        ladder_error,
    }))
}

/// Escalation step `step_id`'s ladder steps, in order. Each reason and
/// judgement is a name bmlib writes, with `judgement`, `n` and `threshold`
/// exactly where the reason names them; on a run under `earlier` rules any
/// non-empty name is read, and a reason today's bmlib does not write with
/// its columns as stored.
fn ladder_steps(
    db: &mut dyn Db,
    step_id: i64,
    earlier: bool,
) -> Result<Vec<StoredLadderStep>, StoreError> {
    let id = step_id;
    let rows = fetch_all(
        db,
        "SELECT position, reason, judgement, n, threshold, levels FROM evidence_ladder_steps
         WHERE step_id = ? ORDER BY position",
        &[Value::from(id)],
    )?;
    rows.iter()
        .enumerate()
        .map(|(index, s)| {
            check_position(id, "ladder step", s, index)?;
            let reason = s.get_str("reason")?.to_string();
            known_unless_earlier(id, "ladder reason", &reason, &REASONS, earlier)?;
            let today_s = REASONS.contains(&reason.as_str());
            let judgement = opt_text(id, s, "judgement")?;
            if let Some(j) = &judgement {
                known_unless_earlier(id, "judgement", j, &JUDGEMENTS, earlier)?;
            }
            let n = u32_of(id, "n", opt_int(id, s, "n")?)?;
            let threshold = u32_of(id, "threshold", opt_int(id, s, "threshold")?)?;
            if today_s
                && (judgement.is_some() != (reason == REASON_RISK_OF_BIAS)
                    || n.is_some() != (reason == REASON_SMALL_TRIAL)
                    || threshold.is_some() != (reason == REASON_SMALL_TRIAL))
            {
                return Err(corrupt(
                    id,
                    format!("a {reason} ladder step's columns do not fit it"),
                ));
            }
            let levels = s.get_i64("levels")?;
            Ok(StoredLadderStep {
                reason,
                judgement,
                n,
                threshold,
                levels: u8::try_from(levels)
                    .map_err(|_| corrupt(id, format!("{levels} levels is out of range")))?,
            })
        })
        .collect()
}

// --- Back into bmlib's types ---

/// bmlib's value named `name` by its serde (a `Judgement`, an `Answer`…).
fn from_wire<T: DeserializeOwned>(name: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(name.to_string()))
        .map_err(|_| format!("{name:?} is not one of bmlib's names"))
}

/// `citations` as bmlib's, each with its wording re-read from `text`.
fn pico_citations(
    text: &CanonicalText,
    citations: &[StoredCitation],
) -> Result<Vec<PicoCitation>, String> {
    citations
        .iter()
        .map(|c| {
            let span = TextSpan {
                start: c.start,
                end: c.end,
            };
            let wording = text
                .span_text(span)
                .ok_or_else(|| format!("span {span} is outside the text"))?;
            Ok(PicoCitation {
                start: c.start,
                end: c.end,
                text: wording.to_string(),
                similarity: c.similarity,
                exact: c.exact,
            })
        })
        .collect()
}

/// `quotes` as bmlib's.
fn pico_contradicted(quotes: &[StoredContradicted]) -> Result<Vec<PicoContradictedQuote>, String> {
    quotes
        .iter()
        .map(|q| {
            Ok(PicoContradictedQuote {
                quote: q.quote.clone(),
                contradiction: contradiction_of(&q.contradiction)?,
            })
        })
        .collect()
}

/// The stored design as bmlib's [`StudyDesign`], read as
/// `DesignProposal::from_json` reads it: a name that does not read back as
/// itself is [`StoreError::Corrupt`].
pub fn study_design(design: &StoredDesign) -> Result<StudyDesign, StoreError> {
    let read = study_design_from_str(&design.design);
    if read.as_str() != design.design {
        return Err(corrupt(
            design.step_id,
            format!("{:?} is not a design", design.design),
        ));
    }
    Ok(read)
}

/// Stored outcome `o` as bmlib's [`ProposedOutcome`], its citations' wording
/// re-read from `text`. What `ProposedOutcome::from_json` refuses (an
/// incoherent result, a result and an error both) is
/// [`StoreError::Corrupt`], naming the outcome's row.
pub fn proposed_outcome(
    o: &StoredOutcome,
    text: &CanonicalText,
) -> Result<ProposedOutcome, StoreError> {
    let bad = |why: String| StoreError::Corrupt(format!("evidence outcome {}: {why}", o.id));
    let result = match &o.result {
        None => None,
        Some(r) => Some(ReportedResult {
            measure: r.measure.clone(),
            scale: from_wire(&r.scale).map_err(bad)?,
            estimate: r.estimate,
            ci_low: r.ci_low,
            ci_high: r.ci_high,
            ci_level: r.ci_level,
            n_analysed: r.n_analysed,
        }),
    };
    let built = ProposedOutcome {
        name: o.name.clone(),
        timepoint: o.timepoint.clone(),
        result,
        result_error: o.result_error.clone(),
        citations: pico_citations(text, &o.quotes.citations).map_err(bad)?,
        quotes_not_found: o.quotes.not_found.clone(),
        contradicted: pico_contradicted(&o.quotes.contradicted).map_err(bad)?,
        quotes_dropped: o.quotes.dropped,
    };
    // bmlib's own reader holds it to what bmlib makes.
    ProposedOutcome::from_json(&built.to_json()).map_err(bad)
}

/// Domain step `step` as bmlib's [`DomainResult`], with `row` its stored
/// assessment ([`stored_domain`]) and citations' wording re-read from `text`.
/// A failed step is `Failed` with its stored failure; a done one is
/// `Assessed`, held by `DomainResult::from_json` to what bmlib makes (answers
/// that give the judgement, verdicts the checks give, D5's answers following
/// from its facts). A pending step, a row that does not fit the step, or what
/// bmlib refuses is [`StoreError::Corrupt`].
pub fn domain_result(
    step: &StoredStep,
    row: Option<&StoredDomain>,
    text: &CanonicalText,
) -> Result<DomainResult, StoreError> {
    let domain = domain_of_step(step)?;
    let bad = |why: String| corrupt(step.id, why);
    match (step.status, &step.failure, row) {
        (StepStatus::Failed, Some(f), None) => Ok(DomainResult::Failed {
            domain,
            reason: f.message.clone(),
            retryable: f.retryable,
            status: f.status,
        }),
        (StepStatus::Done, _, Some(d)) if domain_of(&d.domain) == Some(domain) => {
            let built = DomainResult::Assessed(assessment_of(domain, step, d, text).map_err(bad)?);
            DomainResult::from_json(&built.to_json()).map_err(bad)
        }
        (status, _, row) => Err(bad(format!(
            "a {} step {} a stored assessment of it cannot be handed to bmlib",
            status.as_str(),
            if row.is_some() { "with" } else { "without" }
        ))),
    }
}

/// Stored assessment `d` of `domain` as bmlib's, before bmlib's check.
fn assessment_of(
    domain: Domain,
    step: &StoredStep,
    d: &StoredDomain,
    text: &CanonicalText,
) -> Result<DomainAssessment, String> {
    let answers = d
        .answers
        .iter()
        .map(|a| {
            let answer = MachineAnswer {
                answer: from_wire(&a.answer)?,
                citations: pico_citations(text, &a.quotes.citations)?,
                quotes_not_found: a.quotes.not_found.clone(),
                contradicted: pico_contradicted(&a.quotes.contradicted)?,
                quotes_dropped: a.quotes.dropped,
                reasoning: a.reasoning.clone(),
            };
            Ok((a.question_id.clone(), answer))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let facts =
        d.d5.as_ref()
            .map(|d5| facts_of(d5, text).map(Box::new))
            .transpose()?;
    Ok(DomainAssessment {
        domain,
        answers,
        judgement: from_wire(&d.judgement)?,
        condensed: step.condensed == Some(true),
        ignored: d.ignored.clone(),
        facts,
    })
}

/// Stored D5 facts `d5` as bmlib's, before bmlib's check.
fn facts_of(d5: &StoredD5, text: &CanonicalText) -> Result<D5Facts, String> {
    let facts = d5
        .facts
        .iter()
        .map(|f| {
            let fact =
                D5Fact::parse(&f.fact).ok_or_else(|| format!("unknown fact {:?}", f.fact))?;
            let report = FactReport {
                claimed: f.claimed,
                citations: pico_citations(text, &f.quotes.citations)?,
                quotes_not_found: f.quotes.not_found.clone(),
                contradicted: pico_contradicted(&f.quotes.contradicted)?,
                quotes_dropped: f.quotes.dropped,
                verdict: FactVerdict::parse(&f.verdict)
                    .ok_or_else(|| format!("unknown verdict {:?}", f.verdict))?,
            };
            Ok((fact, report))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let measurements = d5
        .measurements
        .iter()
        .map(|m| {
            Ok(OtherMeasurement {
                label: m.label.clone(),
                named: pico_citations(text, &m.named)?,
                result: pico_citations(text, &m.result)?,
                second_look: m
                    .second_look
                    .as_ref()
                    .map(|look| pico_citations(text, look))
                    .transpose()?,
                second_look_error: m.second_look_error.clone(),
                second_look_retryable: m.second_look_retryable,
                second_look_status: m.second_look_status,
                quotes_not_found: m.not_found.clone(),
                contradicted: pico_contradicted(&m.contradicted)?,
                quotes_dropped: m.dropped,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let a = &d5.analysis;
    Ok(D5Facts {
        outcome: d5.outcome.clone(),
        facts,
        measurements,
        analysis: AnalysisCheck {
            differs: a.differs,
            planned: pico_citations(text, &a.planned)?,
            reported: pico_citations(text, &a.reported)?,
            quotes_not_found: a.not_found.clone(),
            contradicted: pico_contradicted(&a.contradicted)?,
            quotes_dropped: a.dropped,
        },
    })
}

/// The domain steps of run `run_id` for `outcome` (`None`: D1), in domain
/// order whatever their positions.
fn domain_steps(
    db: &mut dyn Db,
    run_id: i64,
    outcome: Option<usize>,
) -> Result<Vec<StoredStep>, StoreError> {
    let (clause, params) = match outcome {
        None => (
            "outcome IS NULL",
            vec![Value::from(run_id), Value::from(STEP_DOMAIN)],
        ),
        Some(o) => (
            "outcome = ?",
            vec![
                Value::from(run_id),
                Value::from(STEP_DOMAIN),
                Value::from(o),
            ],
        ),
    };
    fetch_all(
        db,
        &format!(
            "SELECT {STEP_COLUMNS} FROM evidence_steps
             WHERE run_id = ? AND kind = ? AND {clause} ORDER BY domain"
        ),
        &params,
    )?
    .iter()
    .map(step_from)
    .collect()
}

/// What outcome `outcome`'s escalation needs from run `run_id`, as bmlib's
/// types: the outcome, the trial's D1, and the outcome's D2–D5 in that order
/// (read with an explicit order on the domain). Each domain must be done or
/// failed; its citations' wording is re-read from `text`. No such outcome, D1
/// not once, D2–D5 not each once, a pending domain, or anything
/// [`domain_result`] or [`proposed_outcome`] refuses is
/// [`StoreError::Corrupt`].
pub fn rob2_inputs(
    db: &mut dyn Db,
    run_id: i64,
    outcome: usize,
    text: &CanonicalText,
) -> Result<(ProposedOutcome, DomainResult, Vec<DomainResult>), StoreError> {
    let bad = |why: String| StoreError::Corrupt(format!("evidence run {run_id}: {why}"));
    let outcomes = stored_outcomes(db, run_id)?;
    let stored = outcomes
        .get(outcome)
        .ok_or_else(|| bad(format!("it has no outcome {outcome}")))?;
    let proposed = proposed_outcome(stored, text)?;
    let study = domain_steps(db, run_id, None)?;
    let rest = domain_steps(db, run_id, Some(outcome))?;
    let domains = |steps: &[StoredStep]| -> Vec<Domain> {
        steps
            .iter()
            .filter_map(|s| domain_of_step(s).ok())
            .collect()
    };
    if domains(&study) != [Domain::D1] || domains(&rest) != Domain::ALL[1..] {
        return Err(bad(format!(
            "outcome {outcome}'s domains are {:?} and {:?}, not D1 and D2–D5",
            domains(&study),
            domains(&rest)
        )));
    }
    let mut results = Vec::with_capacity(rest.len() + 1);
    for step in study.iter().chain(&rest) {
        let row = stored_domain(db, step.id)?;
        results.push(domain_result(step, row.as_ref(), text)?);
    }
    let study = results.remove(0);
    Ok((proposed, study, results))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence_store::tests::{
        asked, assessed, assessed_run, d5, domain_step, escalation_step, outcome, outcome_name,
        save_pending, step_id, stored, trial, weighed,
    };
    use crate::evidence_store::{
        run, run_text, save_domain, save_outcomes, EvidenceStep, StoredStep,
    };
    use crate::pico_store::tests::cite;
    use crate::store::open_memory_store;
    use bmlib::db::execute;
    use bmlib::quality::rob2_d5_checks::{FactVerdict, D5_FACTS};
    use bmlib::quality::{
        Answer, EffectScale, Judgement, Level, OutcomeProposal, RctLayout, StudyDesign,
        OUTCOME_PROMPT_VERSION,
    };
    use rusqlite::Connection;

    /// The stored step `step` of run `run_id`.
    fn step_of(c: &mut Connection, run_id: i64, step: EvidenceStep) -> StoredStep {
        stored(c, run_id)
            .steps
            .into_iter()
            .find(|s| s.step == step)
            .unwrap()
    }

    /// Domain step `step` of run `run_id`, read back and handed to bmlib.
    fn read_back(c: &mut Connection, run_id: i64, step: EvidenceStep) -> DomainResult {
        let s = step_of(c, run_id, step);
        let row = stored_domain(c, s.id).unwrap();
        let text = stored(c, run_id).text;
        domain_result(&s, row.as_ref(), &text).unwrap()
    }

    /// The span "Para one." of the test paper, as stored.
    fn para_one(c: &mut Connection, run_id: i64) -> StoredCitation {
        let (p, cid) = run_text(c, run_id).unwrap().unwrap();
        let q = cite(c, &p, cid, "Para one.");
        StoredCitation {
            start: q.start,
            end: q.end,
            similarity: q.similarity,
            exact: q.exact,
        }
    }

    /// A run with D1 and outcome 0's D2–D5 and escalation saved (its D5 with
    /// a claimed fact and a failed second look) reads back every row,
    /// citation and quote list as saved; outcome 1's steps hold nothing yet.
    #[test]
    fn run_round_trips() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let d5_0 = domain_step(Domain::D5, Some(0));
        let mut skip = vec![d5_0];
        skip.extend((2..=5).map(|d| domain_step(Domain::ALL[d - 1], Some(1))));
        skip.push(escalation_step(1));
        save_pending(&mut c, &p, cid, run_id, &skip);
        let quote = cite(&mut c, &p, cid, "Para one.");
        let named = cite(&mut c, &p, cid, "Para two.");
        let rich = d5("Outcome 0", Some(quote.clone()), Some(named));
        let d5_id = step_id(&mut c, run_id, d5_0);
        save_domain(&mut c, d5_id, &rich).unwrap();
        let text = stored(&mut c, run_id).text;

        let design = stored_design(&mut c, run_id).unwrap().unwrap();
        assert_eq!(
            design.step_id,
            step_id(&mut c, run_id, EvidenceStep::Design)
        );
        assert_eq!(
            (
                design.design.as_str(),
                design.rct_layout.as_deref(),
                design.reasoning.as_str()
            ),
            ("rct", Some("parallel"), "Randomised.")
        );
        assert_eq!(
            design.quotes,
            StoredQuotes {
                citations: vec![para_one(&mut c, run_id)],
                not_found: vec!["Nowhere.".into()],
                contradicted: vec![StoredContradicted {
                    quote: "Para three.".into(),
                    contradiction: "number".into(),
                }],
                dropped: 1,
            }
        );
        assert_eq!(study_design(&design).unwrap(), StudyDesign::Rct);

        let outcomes = stored_outcomes(&mut c, run_id).unwrap();
        assert_eq!(outcomes.len(), 2);
        for (i, o) in outcomes.iter().enumerate() {
            assert_eq!(o.position, i);
            let saved = outcome(&mut c, &p, cid, i);
            assert_eq!(proposed_outcome(o, &text).unwrap(), saved);
        }
        let result = outcomes[0].result.as_ref().unwrap();
        assert_eq!(
            (result.scale.as_str(), result.n_analysed),
            ("difference", Some(312))
        );

        let one = cite(&mut c, &p, cid, "Para one.");
        let d1 = domain_step(Domain::D1, None);
        assert_eq!(
            read_back(&mut c, run_id, d1),
            assessed(Domain::D1, Answer::Y, Some(one.clone()))
        );
        for d in [Domain::D2, Domain::D3, Domain::D4] {
            let got = read_back(&mut c, run_id, domain_step(d, Some(0)));
            assert_eq!(got, assessed(d, Answer::Y, Some(one.clone())), "{d:?}");
        }
        assert_eq!(read_back(&mut c, run_id, d5_0), rich);
        let row = stored_domain(&mut c, d5_id).unwrap().unwrap();
        let facts = row.d5.as_ref().unwrap();
        assert_eq!(facts.outcome, "Outcome 0");
        assert_eq!(facts.facts.len(), D5_FACTS.len());
        let looks: Vec<(&str, Option<usize>, Option<&str>)> = facts
            .measurements
            .iter()
            .map(|m| {
                (
                    m.label.as_str(),
                    m.second_look.as_ref().map(Vec::len),
                    m.second_look_error.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            looks,
            vec![
                ("at 6 months", None, Some("the server is busy")),
                ("at 3 months", Some(0), None)
            ]
        );
        let d2 = step_id(&mut c, run_id, domain_step(Domain::D2, Some(0)));
        assert!(stored_domain(&mut c, d2).unwrap().unwrap().d5.is_none());

        let e0 = step_id(&mut c, run_id, escalation_step(0));
        let got = stored_outcome_result(&mut c, e0).unwrap().unwrap();
        let ladder = weighed(&outcome(&mut c, &p, cid, 0), &asked())
            .ladder
            .unwrap();
        assert_eq!(
            (
                got.escalation.as_str(),
                got.escalate,
                got.escalation_reasoning.as_deref(),
                got.overall.as_deref(),
                got.ladder_error.as_deref()
            ),
            (
                "asked",
                Some(false),
                Some("Not serious."),
                Some("some_concerns"),
                None
            )
        );
        let stored_ladder = got.ladder.unwrap();
        assert_eq!(
            (stored_ladder.start.as_str(), stored_ladder.weight),
            ("high", ladder.weight)
        );
        assert_eq!(stored_ladder.steps.len(), ladder.steps.len());
        assert_eq!(
            (
                stored_ladder.steps[0].reason.as_str(),
                stored_ladder.steps[0].judgement.as_deref()
            ),
            ("risk_of_bias", Some("some_concerns"))
        );
        let e1 = step_id(&mut c, run_id, escalation_step(1));
        assert_eq!(stored_outcome_result(&mut c, e1).unwrap(), None);
        let d2_1 = step_id(&mut c, run_id, domain_step(Domain::D2, Some(1)));
        assert_eq!(stored_domain(&mut c, d2_1).unwrap(), None);

        let (proposed, study, domains) = rob2_inputs(&mut c, run_id, 0, &text).unwrap();
        assert_eq!(proposed, outcome(&mut c, &p, cid, 0));
        assert_eq!(study, assessed(Domain::D1, Answer::Y, Some(one.clone())));
        let mut expected: Vec<DomainResult> = [Domain::D2, Domain::D3, Domain::D4]
            .into_iter()
            .map(|d| assessed(d, Answer::Y, Some(one.clone())))
            .collect();
        expected.push(rich);
        assert_eq!(domains, expected);
        assert!(matches!(
            rob2_inputs(&mut c, run_id, 1, &text),
            Err(StoreError::Corrupt(_))
        ));
        assert!(matches!(
            rob2_inputs(&mut c, run_id, 2, &text),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// An outcome's domains are read in domain order whatever their row ids
    /// and positions: rows stored D5, D3, D2, D4 give D2–D5.
    #[test]
    fn domains_read_back_in_order() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        for (k, domain) in ["D5", "D3", "D2", "D4"].into_iter().enumerate() {
            execute(
                &mut c,
                "UPDATE evidence_steps SET id = ?, position = ?
                 WHERE run_id = ? AND kind = 'domain' AND domain = ? AND outcome = 0",
                &[
                    Value::from(10_000 + k),
                    Value::from(1_000 + k),
                    Value::from(run_id),
                    Value::from(domain),
                ],
            )
            .unwrap();
        }
        save_pending(&mut c, &p, cid, run_id, &[]);
        let moved = run(&mut c, run_id).unwrap_err();
        assert!(matches!(moved, StoreError::Corrupt(_)), "positions moved");
        let (paper, canonical) = run_text(&mut c, run_id).unwrap().unwrap();
        let text = crate::store::canonical_by_id(&mut c, &paper, canonical)
            .unwrap()
            .unwrap()
            .text;
        let (_, _, domains) = rob2_inputs(&mut c, run_id, 0, &text).unwrap();
        let order: Vec<Domain> = domains.iter().map(DomainResult::domain).collect();
        assert_eq!(order, Domain::ALL[1..].to_vec());
    }

    /// A damaged row is `Corrupt`, never a panic: an unknown question id, a
    /// judgement bmlib never named, facts on a domain that is not D5, a done
    /// domain or escalation with no row, a plan with a step missing.
    #[test]
    fn a_damaged_row_is_corrupt() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        save_pending(&mut c, &p, cid, run_id, &[]);
        let text = stored(&mut c, run_id).text;
        let corrupt = |r: Result<Option<StoredDomain>, StoreError>| {
            assert!(matches!(r, Err(StoreError::Corrupt(_))), "{r:?}");
        };
        let d1 = step_id(&mut c, run_id, domain_step(Domain::D1, None));
        execute(
            &mut c,
            "UPDATE evidence_answers SET question_id = '9.9' WHERE step_id = ? AND question_id = '1.1'",
            &[Value::from(d1)],
        )
        .unwrap();
        corrupt(stored_domain(&mut c, d1));
        assert!(matches!(
            rob2_inputs(&mut c, run_id, 0, &text),
            Err(StoreError::Corrupt(_))
        ));
        let d2 = step_id(&mut c, run_id, domain_step(Domain::D2, Some(0)));
        execute(
            &mut c,
            "UPDATE evidence_domains SET judgement = 'awful' WHERE step_id = ?",
            &[Value::from(d2)],
        )
        .unwrap();
        corrupt(stored_domain(&mut c, d2));
        let d3 = step_id(&mut c, run_id, domain_step(Domain::D3, Some(0)));
        execute(
            &mut c,
            "UPDATE evidence_domains SET facts_outcome = 'x' WHERE step_id = ?",
            &[Value::from(d3)],
        )
        .unwrap();
        corrupt(stored_domain(&mut c, d3));
        let d4 = step_id(&mut c, run_id, domain_step(Domain::D4, Some(0)));
        execute(
            &mut c,
            "DELETE FROM evidence_domains WHERE step_id = ?",
            &[Value::from(d4)],
        )
        .unwrap();
        corrupt(stored_domain(&mut c, d4));
        let e1 = step_id(&mut c, run_id, escalation_step(1));
        execute(
            &mut c,
            "DELETE FROM evidence_outcome_results WHERE step_id = ?",
            &[Value::from(e1)],
        )
        .unwrap();
        assert!(matches!(
            stored_outcome_result(&mut c, e1),
            Err(StoreError::Corrupt(_))
        ));
        let d4_1 = step_id(&mut c, run_id, domain_step(Domain::D4, Some(1)));
        execute(
            &mut c,
            "DELETE FROM evidence_steps WHERE id = ?",
            &[Value::from(d4_1)],
        )
        .unwrap();
        assert!(matches!(run(&mut c, run_id), Err(StoreError::Corrupt(_))));
    }

    /// A D5 measurement or a ladder step out of position is `Corrupt`; so is
    /// a contradiction bmlib never named on a current run, while an
    /// earlier-rules run reads it as stored (an empty one is refused on
    /// either).
    #[test]
    fn moved_positions_and_unknown_contradictions_are_corrupt() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let d5_0 = domain_step(Domain::D5, Some(0));
        save_pending(&mut c, &p, cid, run_id, &[d5_0]);
        let named = cite(&mut c, &p, cid, "Para two.");
        let d5_id = step_id(&mut c, run_id, d5_0);
        save_domain(&mut c, d5_id, &d5(&outcome_name(0), None, Some(named))).unwrap();
        let e0 = step_id(&mut c, run_id, escalation_step(0));
        assert!(!stored_outcome_result(&mut c, e0)
            .unwrap()
            .unwrap()
            .ladder
            .unwrap()
            .steps
            .is_empty());
        let shift = |c: &mut Connection, table: &str, step: i64, by: i64| {
            execute(
                c,
                &format!("UPDATE {table} SET position = position + ? WHERE step_id = ?"),
                &[Value::from(by), Value::from(step)],
            )
            .unwrap();
        };
        shift(&mut c, "evidence_d5_measurements", d5_id, 10);
        let moved = stored_domain(&mut c, d5_id).unwrap_err();
        assert!(
            matches!(&moved, StoreError::Corrupt(m) if m.contains("measurement 0 is at position 10"))
        );
        shift(&mut c, "evidence_d5_measurements", d5_id, -10);
        assert!(stored_domain(&mut c, d5_id).is_ok());
        shift(&mut c, "evidence_ladder_steps", e0, 10);
        let moved = stored_outcome_result(&mut c, e0).unwrap_err();
        assert!(matches!(&moved, StoreError::Corrupt(m) if m.contains("ladder step 0")));

        let contradiction = |c: &mut Connection, name: &str| {
            let json = format!(r#"[{{"quote":"q","contradiction":"{name}"}}]"#);
            execute(
                c,
                "UPDATE evidence_designs SET quotes_contradicted_json = ?",
                &[Value::from(json)],
            )
            .unwrap();
            stored_design(c, run_id)
        };
        assert!(matches!(
            contradiction(&mut c, "sarcasm"),
            Err(StoreError::Corrupt(_))
        ));
        execute(
            &mut c,
            "UPDATE evidence_runs SET algorithm_version = 'earlier' WHERE id = ?",
            &[Value::from(run_id)],
        )
        .unwrap();
        let design = contradiction(&mut c, "sarcasm").unwrap().unwrap();
        assert_eq!(design.quotes.contradicted[0].contradiction, "sarcasm");
        assert!(matches!(
            contradiction(&mut c, ""),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// A question id today's bmlib does not know reads back on a run made
    /// under earlier rules (answered and ignored), and is `Corrupt` once the
    /// run's rules are current again; an empty id is refused on either.
    #[test]
    fn an_earlier_run_keeps_its_question_ids() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        save_pending(&mut c, &p, cid, run_id, &[]);
        let d1 = step_id(&mut c, run_id, domain_step(Domain::D1, None));
        execute(
            &mut c,
            "UPDATE evidence_answers SET question_id = '9.9' WHERE step_id = ? AND question_id = '1.1'",
            &[Value::from(d1)],
        )
        .unwrap();
        execute(
            &mut c,
            "UPDATE evidence_domains SET ignored_json = '[\"9.8\"]' WHERE step_id = ?",
            &[Value::from(d1)],
        )
        .unwrap();
        let set_version = |c: &mut Connection, v: i64| {
            execute(
                c,
                "UPDATE evidence_runs SET rob2_prompt_version = ? WHERE id = ?",
                &[Value::from(v), Value::from(run_id)],
            )
            .unwrap();
        };
        let current = i64::from(crate::evidence_store::RunVersions::current().rob2_prompt);
        set_version(&mut c, current + 1);
        let got = stored_domain(&mut c, d1).unwrap().unwrap();
        assert!(got.answers.iter().any(|a| a.question_id == "9.9"));
        assert_eq!(got.ignored, vec!["9.8".to_string()]);
        set_version(&mut c, current);
        assert!(matches!(
            stored_domain(&mut c, d1),
            Err(StoreError::Corrupt(_))
        ));
        set_version(&mut c, current + 1);
        execute(
            &mut c,
            "UPDATE evidence_answers SET question_id = '' WHERE step_id = ? AND question_id = '9.9'",
            &[Value::from(d1)],
        )
        .unwrap();
        assert!(matches!(
            stored_domain(&mut c, d1),
            Err(StoreError::Corrupt(_))
        ));
    }

    /// What was saved comes back as bmlib's own types, equal to it: a D5
    /// with facts and second looks, outcomes with and without a result, a
    /// failed domain; a pending one cannot be handed back.
    #[test]
    fn the_conversion_gives_bmlib_s_types() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let text = stored(&mut c, run_id).text;
        let quote = cite(&mut c, &p, cid, "Para one.");
        let named = cite(&mut c, &p, cid, "Para two.");
        let rich = d5("Outcome 0", Some(quote), Some(named));
        let d5_0 = domain_step(Domain::D5, Some(0));
        let id = step_id(&mut c, run_id, d5_0);
        save_domain(&mut c, id, &rich).unwrap();
        assert_eq!(read_back(&mut c, run_id, d5_0), rich);

        let failed = DomainResult::Failed {
            domain: Domain::D3,
            reason: "the server is busy".into(),
            retryable: true,
            status: Some(503),
        };
        let d3 = domain_step(Domain::D3, Some(1));
        let id = step_id(&mut c, run_id, d3);
        save_domain(&mut c, id, &failed).unwrap();
        assert_eq!(read_back(&mut c, run_id, d3), failed);
        let pending = step_of(&mut c, run_id, domain_step(Domain::D2, Some(1)));
        assert!(matches!(
            domain_result(&pending, None, &text),
            Err(StoreError::Corrupt(_))
        ));

        // A second run whose outcomes are one without a result.
        let other = crate::evidence_store::create_run(&mut c, &p, cid, "m").unwrap();
        save_pending(&mut c, &p, cid, other, &[EvidenceStep::Outcomes]);
        let mut refused = outcome(&mut c, &p, cid, 0);
        refused.result = None;
        refused.result_error = Some("ci_low <= estimate <= ci_high does not hold".into());
        let proposal = OutcomeProposal {
            outcomes: vec![refused.clone()],
            condensed: false,
            prompt_version: OUTCOME_PROMPT_VERSION,
        };
        let step = step_id(&mut c, other, EvidenceStep::Outcomes);
        save_outcomes(&mut c, step, &proposal).unwrap();
        let back = stored_outcomes(&mut c, other).unwrap();
        assert_eq!(back[0].result, None);
        assert_eq!(proposed_outcome(&back[0], &text).unwrap(), refused);
        let mut design = stored_design(&mut c, other).unwrap().unwrap();
        assert_eq!(
            trial(&mut c, &p, cid).design,
            study_design(&design).unwrap()
        );
        design.design = "randomised controlled trial".into();
        assert!(matches!(study_design(&design), Err(StoreError::Corrupt(_))));
    }

    /// The designs, layouts, D5 facts and verdicts, judgements, answers,
    /// levels and scales the readers know are bmlib's own, so every value
    /// bmlib writes reads back; a bmlib bump that adds one fails here. (The
    /// ladder reasons are pinned by the writer's match on `LadderReason`.)
    #[test]
    fn the_spellings_are_bmlib_s() {
        assert_eq!(
            DESIGNS.to_vec(),
            StudyDesign::ALL.map(StudyDesign::as_str).to_vec()
        );
        let layouts = [
            RctLayout::Parallel,
            RctLayout::Cluster,
            RctLayout::Crossover,
            RctLayout::Other,
        ];
        assert_eq!(
            RCT_LAYOUTS.to_vec(),
            layouts.map(RctLayout::as_str).to_vec()
        );
        assert_eq!(FACTS.to_vec(), D5_FACTS.map(|f| f.as_str()).to_vec());
        assert_eq!(
            VERDICTS.to_vec(),
            FactVerdict::ALL.map(FactVerdict::as_str).to_vec()
        );
        let judgements = [Judgement::Low, Judgement::SomeConcerns, Judgement::High];
        assert_eq!(
            JUDGEMENTS.to_vec(),
            judgements
                .iter()
                .map(|j| wire(j).unwrap())
                .collect::<Vec<_>>()
        );
        let answers = [Answer::Y, Answer::Py, Answer::Pn, Answer::N, Answer::Ni];
        assert_eq!(
            ANSWERS.to_vec(),
            answers.iter().map(|a| wire(a).unwrap()).collect::<Vec<_>>()
        );
        let levels = [Level::High, Level::Moderate, Level::Low, Level::VeryLow];
        assert_eq!(
            LEVELS.to_vec(),
            levels.iter().map(|l| wire(l).unwrap()).collect::<Vec<_>>()
        );
        let scales = [EffectScale::Ratio, EffectScale::Difference];
        assert_eq!(
            SCALES.to_vec(),
            scales.iter().map(|s| wire(s).unwrap()).collect::<Vec<_>>()
        );
    }
}
