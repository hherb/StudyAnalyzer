//! The Evidence weight tab's views (design 2026-11-24, §7): stored runs as
//! the serialisable shapes the frontend shows, spans in UTF-16 units. The
//! PRISMA tab's counterparts are in [`crate::prisma_view`].
//!
//! A view reads its run's rows as text ([`crate::evidence_rows`]'s
//! `stored_*`), never as bmlib's types, so a run made under earlier rules is
//! shown in full, as it was made. bmlib is asked only for wording: a
//! question's text, a domain's title and a fact's meaning, each for a name it
//! knows; any other name is shown as stored.
//!
//! A failure is never a score: a pending or failed domain has no judgement,
//! and a run that is not complete has no paper weight.

use bmlib::db::{transaction_with, Db, DbError};
use bmlib::fulltext::canonical::{CanonicalText, TextSpan};
use bmlib::quality::rob2_d5_checks::D5Fact;
use bmlib::quality::rob2_d5_facts::fact_meaning;
use bmlib::quality::rob2_questions::{domain_title, question_text};
use bmlib::quality::{questions, Domain, Rob2Step};
use serde::Serialize;

use crate::evidence::step_label;
use crate::evidence_rows::{
    stored_design, stored_domain, stored_outcome_result, stored_outcomes, StoredCitation,
    StoredContradicted, StoredDesign, StoredLadder, StoredOutcome, StoredQuotes, StoredResult,
};
use crate::evidence_store::{
    domain_name, latest_run, run as stored_run, runs, EvidenceRunSummary, EvidenceStep, RunKind,
    RunStatus, StepStatus, StoredEvidenceRun, StoredStep,
};
use crate::prisma_view::PrismaCitationView;
use crate::store::{latest_canonical_id, PaperId, StoreError};
use crate::view::{span_to_utf16, utf16_offsets, ContradictedQuoteView};

/// The quotes backing one claim: the paper's spans and what was lost.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuotesView {
    /// The paper's spans, in the order given; each one's `text` is the
    /// paper's wording, never the model's copy.
    pub citations: Vec<PrismaCitationView>,
    /// The model's quotes not found in the paper, verbatim.
    pub quotes_not_found: Vec<String>,
    /// The model's quotes the paper contradicts, with why.
    pub quotes_contradicted: Vec<ContradictedQuoteView>,
    /// How many of the model's quotes were not usable text at all.
    pub quotes_dropped: usize,
}

/// The study design as the tab shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DesignView {
    /// bmlib's name for the design (`rct`, `cohort_prospective`, …).
    pub design: String,
    /// A randomised trial's layout (`parallel`, `cluster`, …).
    pub rct_layout: Option<String>,
    /// The model's explanation.
    pub reasoning: String,
    /// The quotes backing it.
    pub quotes: QuotesView,
}

/// An outcome's reported effect.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResultView {
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
    /// The interval's level, as a fraction (`0.95`).
    pub ci_level: Option<f64>,
    /// How many were analysed.
    pub n_analysed: Option<u32>,
}

/// One answer to a signalling question.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnswerView {
    /// The question's id (`2.3`).
    pub question_id: String,
    /// The question's wording (bmlib's `question_text`), or the id itself
    /// for a question today's bmlib does not ask (a run under earlier rules).
    pub question: String,
    /// The answer (`Y`, `PY`, `PN`, `N`, `NI`).
    pub answer: String,
    /// The model's explanation (D5's: the facts it was derived from).
    pub reasoning: String,
    /// The quotes backing it.
    pub quotes: QuotesView,
}

/// One D5 fact.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FactView {
    /// The fact's name (`plan_cited`, …).
    pub fact: String,
    /// What it means (bmlib's `fact_meaning`), or the name for a fact today's
    /// bmlib does not know.
    pub meaning: String,
    /// Whether the model said it is present.
    pub claimed: bool,
    /// Whether it counts, and if not, why (`present`, `not_claimed`, …).
    pub verdict: String,
    /// The quotes backing it.
    pub quotes: QuotesView,
}

/// One other measurement of D5's outcome.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeasurementView {
    /// The model's short name for it.
    pub label: String,
    /// The passages naming it.
    pub named: Vec<PrismaCitationView>,
    /// The passages giving its result.
    pub result: Vec<PrismaCitationView>,
    /// What a second look found; `None` when none was made.
    pub second_look: Option<Vec<PrismaCitationView>>,
    /// Why the second look failed, when it did.
    pub second_look_error: Option<String>,
    /// Its quotes that were lost (its `citations` are always empty: its
    /// passages are `named` and `result`).
    pub quotes: QuotesView,
}

/// D5's check of the analysis reported against the one planned.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnalysisView {
    /// Whether the model says they differ with no reason given.
    pub differs: bool,
    /// The passages naming the planned main analysis.
    pub planned: Vec<PrismaCitationView>,
    /// The passages giving the result with its analysis.
    pub reported: Vec<PrismaCitationView>,
    /// Its quotes that were lost (its `citations` are always empty).
    pub quotes: QuotesView,
}

/// One RoB 2 domain as the tab shows it, whatever its step's status.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DomainView {
    /// The domain (`D1` … `D5`).
    pub domain: String,
    /// Its title (bmlib's `domain_title`).
    pub name: String,
    /// Where its step stands (`pending`, `done` or `failed`).
    pub status: StepStatus,
    /// Why its step failed; kept on a step set back to pending until it is
    /// saved.
    pub error: Option<String>,
    /// Its judgement (`low`, `some_concerns`, `high`); `None` unless done,
    /// which the tab shows as *Not assessed*.
    pub judgement: Option<String>,
    /// Whether it was judged from a condensed paper.
    pub condensed: bool,
    /// The answers, by question id.
    pub answers: Vec<AnswerView>,
    /// Question ids answered although they did not apply.
    pub ignored: Vec<String>,
    /// D5's facts, by name; `None` for every other domain and a D5 not done.
    pub facts: Option<Vec<FactView>>,
    /// D5's other measurements, in the reply's order; empty otherwise.
    pub measurements: Vec<MeasurementView>,
    /// D5's analysis check; `None` otherwise.
    pub analysis: Option<AnalysisView>,
}

/// One step down an outcome's ladder.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LadderStepView {
    /// Why (`risk_of_bias`, `crosses_no_effect`, `small_trial`,
    /// `no_interval`, `sample_size_unknown`).
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

/// An outcome's ladder.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LadderView {
    /// Where it started (`high`, `moderate`, `low`, `very_low`).
    pub start: String,
    /// Where it ended.
    pub level: String,
    /// The end level's weight: the outcome's number.
    pub weight: f64,
    /// Every step, risk of bias first.
    pub steps: Vec<LadderStepView>,
}

/// An escalation the model answered.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EscalationView {
    /// Whether the concerns add up to a high risk of bias.
    pub escalate: bool,
    /// The model's explanation.
    pub reasoning: String,
}

/// Something that failed, named for the user: a step (by its progress label)
/// or a second look (by its measurement's label).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NamedErrorView {
    /// What failed.
    pub label: String,
    /// Why, as stored.
    pub error: String,
}

/// One primary outcome as its card shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutcomeView {
    /// Its index among the run's outcomes, from 0.
    pub index: usize,
    /// The outcome as the paper names it.
    pub name: String,
    /// When it was measured.
    pub timepoint: Option<String>,
    /// Its reported effect.
    pub result: Option<ResultView>,
    /// Why its reported effect was refused.
    pub result_error: Option<String>,
    /// The quotes naming it.
    pub quotes: QuotesView,
    /// Its domains, always D2–D5 in order.
    pub domains: Vec<DomainView>,
    /// The escalation, when the model answered one.
    pub escalation: Option<EscalationView>,
    /// Why its escalation step failed; kept on one set back to pending.
    pub escalation_error: Option<String>,
    /// The overall judgement; `None` for *Not assessed*.
    pub overall: Option<String>,
    /// Its ladder, when one was built; its weight is the outcome's number.
    pub ladder: Option<LadderView>,
    /// Why there is no ladder, when its escalation stored one.
    pub ladder_error: Option<String>,
    /// D5's other measurements whose second look failed, to name beside the
    /// number.
    pub second_look_failures: Vec<NamedErrorView>,
}

/// A stored evidence run as the tab shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvidenceRunView {
    /// The run's id.
    pub id: i64,
    /// The paper text its spans point into.
    pub canonical_id: i64,
    /// The `provider:model` that ran it.
    pub model: String,
    /// When it was started (UTC, `YYYY-MM-DD HH:MM:SS`).
    pub created_at: String,
    /// When the tab first showed it with a RoB 2 result.
    pub first_shown_at: Option<String>,
    /// `incomplete` or `complete`.
    pub status: RunStatus,
    /// What it found (`not_covered`, `no_primary_outcome`, `assessed`), once
    /// known.
    pub kind: Option<RunKind>,
    /// Whether it was made under earlier rules: shown in full, Re-run only.
    pub earlier_rules: bool,
    /// Whether an attempt stopped because its stored outcomes left no room
    /// for the paper in its window, with the context-window setting unchanged
    /// since: Re-run only (#166).
    pub too_long: bool,
    /// Whether any step was judged from a condensed paper.
    pub condensed: bool,
    /// The study design, once its step is done.
    pub design: Option<DesignView>,
    /// The primary outcomes, in order.
    pub outcomes: Vec<OutcomeView>,
    /// D1, judged once for the trial; `None` until the outcomes are stored
    /// (they add D1's step).
    pub study: Option<DomainView>,
    /// How many outcomes have a ladder.
    pub assessed_outcomes: usize,
    /// The largest ladder weight among the outcomes (bmlib's
    /// `paper_weight`), only for a complete, assessed run with every step
    /// done; `None` otherwise.
    pub paper_weight: Option<f64>,
    /// Every step that failed (or was set back with its error kept), by its
    /// progress label.
    pub failed_steps: Vec<NamedErrorView>,
    /// Whether a domain step is done: a RoB 2 result is on show.
    pub has_rob2_result: bool,
}

/// Everything the Evidence weight tab opens with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvidenceView {
    /// The paper's latest canonical text, the one a run goes to.
    pub canonical_id: i64,
    /// The newest run on that text, if there is one and it can be read.
    pub current: Option<EvidenceRunView>,
    /// Why the newest run on that text cannot be read (a row it refuses, as
    /// [`StoreError::Corrupt`]'s message, or a value of the wrong type, as
    /// [`DbError::Column`]'s), with `current` then `None`, so the
    /// tab still offers a new run and the earlier ones (#165). The run stays
    /// stored.
    pub current_error: Option<String>,
    /// Every other run, newest first.
    pub earlier: Vec<EvidenceRunSummary>,
}

/// The wording of `domain`'s question `id`: bmlib's for one of its ids, the
/// id itself for any other (a question an earlier-rules run asked, which
/// today's bmlib does not), since `question_text` panics on those.
fn question_of(domain: Domain, id: &str) -> String {
    if questions(domain).contains(&id) {
        question_text(id).to_string()
    } else {
        id.to_string()
    }
}

/// What fact `fact` means, in bmlib's words; a name bmlib does not know is
/// shown as stored.
fn meaning_of(fact: &str) -> String {
    D5Fact::parse(fact).map_or_else(|| fact.to_string(), |f| fact_meaning(f).to_string())
}

/// A run's text, to show its stored spans: each checked against the text,
/// its wording re-read from it, and its offsets put in UTF-16 units.
struct RunText<'a> {
    /// The run, to name in an error.
    run_id: i64,
    /// The run's canonical text's id, to name in an error.
    canonical_id: i64,
    /// The run's canonical text.
    text: &'a CanonicalText,
    /// [`utf16_offsets`] of `text`.
    offsets: Vec<usize>,
}

impl RunText<'_> {
    /// Citation `c` as shown.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when its span is outside the text: the
    /// readers do not check spans against it, so this is damaged data.
    fn citation(&self, c: &StoredCitation) -> Result<PrismaCitationView, StoreError> {
        let span = TextSpan {
            start: c.start,
            end: c.end,
        };
        let words = self.text.span_text(span).ok_or_else(|| {
            StoreError::Corrupt(format!(
                "evidence run {}: citation {span} is outside canonical text {}",
                self.run_id, self.canonical_id
            ))
        })?;
        Ok(PrismaCitationView {
            span: span_to_utf16(&self.offsets, span),
            text: words.to_string(),
            similarity: c.similarity,
            exact: c.exact,
        })
    }

    /// Every citation of `cs` as shown, in order; errors as [`Self::citation`].
    fn citations(&self, cs: &[StoredCitation]) -> Result<Vec<PrismaCitationView>, StoreError> {
        cs.iter().map(|c| self.citation(c)).collect()
    }

    /// A claim's quotes from their parts; errors as [`Self::citation`].
    fn quotes(
        &self,
        citations: &[StoredCitation],
        not_found: Vec<String>,
        contradicted: Vec<StoredContradicted>,
        dropped: usize,
    ) -> Result<QuotesView, StoreError> {
        Ok(QuotesView {
            citations: self.citations(citations)?,
            quotes_not_found: not_found,
            quotes_contradicted: contradicted
                .into_iter()
                .map(|q| ContradictedQuoteView {
                    quote: q.quote,
                    contradiction: q.contradiction,
                })
                .collect(),
            quotes_dropped: dropped,
        })
    }

    /// Stored quotes `q` as shown; errors as [`Self::citation`].
    fn stored_quotes(&self, q: StoredQuotes) -> Result<QuotesView, StoreError> {
        self.quotes(&q.citations, q.not_found, q.contradicted, q.dropped)
    }
}

/// A stored design as shown; errors as [`RunText::citation`].
fn design_view(text: &RunText, d: StoredDesign) -> Result<DesignView, StoreError> {
    Ok(DesignView {
        design: d.design,
        rct_layout: d.rct_layout,
        reasoning: d.reasoning,
        quotes: text.stored_quotes(d.quotes)?,
    })
}

/// A stored result as shown.
fn result_view(r: StoredResult) -> ResultView {
    ResultView {
        measure: r.measure,
        scale: r.scale,
        estimate: r.estimate,
        ci_low: r.ci_low,
        ci_high: r.ci_high,
        ci_level: r.ci_level,
        n_analysed: r.n_analysed,
    }
}

/// A stored ladder as shown.
fn ladder_view(l: StoredLadder) -> LadderView {
    LadderView {
        start: l.start,
        level: l.level,
        weight: l.weight,
        steps: l
            .steps
            .into_iter()
            .map(|s| LadderStepView {
                reason: s.reason,
                judgement: s.judgement,
                n: s.n,
                threshold: s.threshold,
                levels: s.levels,
            })
            .collect(),
    }
}

/// Domain `domain`'s step `step` as shown: its status and error, and once it
/// is done its judgement, answers and (D5's) facts.
///
/// # Errors
///
/// What [`stored_domain`] refuses, and a span outside the text, as
/// [`StoreError::Corrupt`]; any other store failure as [`StoreError`].
fn domain_view(
    db: &mut dyn Db,
    text: &RunText,
    step: &StoredStep,
    domain: Domain,
) -> Result<DomainView, StoreError> {
    let mut view = DomainView {
        domain: domain_name(domain).to_string(),
        name: domain_title(domain).to_string(),
        status: step.status,
        error: step.failure.as_ref().map(|f| f.message.clone()),
        judgement: None,
        condensed: step.condensed == Some(true),
        answers: vec![],
        ignored: vec![],
        facts: None,
        measurements: vec![],
        analysis: None,
    };
    let Some(d) = stored_domain(db, step.id)? else {
        return Ok(view);
    };
    view.judgement = Some(d.judgement);
    view.ignored = d.ignored;
    view.answers = d
        .answers
        .into_iter()
        .map(|a| {
            Ok(AnswerView {
                question: question_of(domain, &a.question_id),
                question_id: a.question_id,
                answer: a.answer,
                reasoning: a.reasoning,
                quotes: text.stored_quotes(a.quotes)?,
            })
        })
        .collect::<Result<_, StoreError>>()?;
    let Some(d5) = d.d5 else {
        return Ok(view);
    };
    view.facts = Some(
        d5.facts
            .into_iter()
            .map(|f| {
                Ok(FactView {
                    meaning: meaning_of(&f.fact),
                    fact: f.fact,
                    claimed: f.claimed,
                    verdict: f.verdict,
                    quotes: text.stored_quotes(f.quotes)?,
                })
            })
            .collect::<Result<_, StoreError>>()?,
    );
    view.measurements = d5
        .measurements
        .into_iter()
        .map(|m| {
            Ok(MeasurementView {
                named: text.citations(&m.named)?,
                result: text.citations(&m.result)?,
                second_look: m
                    .second_look
                    .as_deref()
                    .map(|cs| text.citations(cs))
                    .transpose()?,
                second_look_error: m.second_look_error,
                quotes: text.quotes(&[], m.not_found, m.contradicted, m.dropped)?,
                label: m.label,
            })
        })
        .collect::<Result<_, StoreError>>()?;
    let a = d5.analysis;
    view.analysis = Some(AnalysisView {
        differs: a.differs,
        planned: text.citations(&a.planned)?,
        reported: text.citations(&a.reported)?,
        quotes: text.quotes(&[], a.not_found, a.contradicted, a.dropped)?,
    });
    Ok(view)
}

/// The step `step` of `run`.
///
/// # Errors
///
/// [`StoreError::Corrupt`] when the run has none: `run` checked its plan, so
/// this is damaged data.
fn step_in(run: &StoredEvidenceRun, step: EvidenceStep) -> Result<&StoredStep, StoreError> {
    run.steps.iter().find(|s| s.step == step).ok_or_else(|| {
        StoreError::Corrupt(format!("evidence run {}: it has no step {step:?}", run.id))
    })
}

/// Outcome `o` of `run` as its card shows it: D2–D5, its escalation, overall
/// judgement and ladder, and D5's failed second looks.
///
/// # Errors
///
/// A step missing from the run, a row the readers refuse, or a span outside
/// the text, as [`StoreError::Corrupt`]; any other store failure as
/// [`StoreError`].
fn outcome_view(
    db: &mut dyn Db,
    text: &RunText,
    run: &StoredEvidenceRun,
    o: StoredOutcome,
) -> Result<OutcomeView, StoreError> {
    let index = o.position;
    let mut domains = Vec::new();
    for domain in Domain::ALL.into_iter().filter(|d| *d != Domain::D1) {
        let step = step_in(
            run,
            EvidenceStep::Rob2(Rob2Step::Domain {
                domain,
                outcome: Some(index),
            }),
        )?;
        domains.push(domain_view(db, text, step, domain)?);
    }
    let escalation_step = step_in(
        run,
        EvidenceStep::Rob2(Rob2Step::Escalation { outcome: index }),
    )?;
    let stored = stored_outcome_result(db, escalation_step.id)?;
    let second_look_failures = domains
        .iter()
        .flat_map(|d| &d.measurements)
        .filter_map(|m| {
            m.second_look_error.as_ref().map(|error| NamedErrorView {
                label: m.label.clone(),
                error: error.clone(),
            })
        })
        .collect();
    let mut view = OutcomeView {
        index,
        name: o.name,
        timepoint: o.timepoint,
        result: o.result.map(result_view),
        result_error: o.result_error,
        quotes: text.stored_quotes(o.quotes)?,
        domains,
        escalation: None,
        escalation_error: escalation_step.failure.as_ref().map(|f| f.message.clone()),
        overall: None,
        ladder: None,
        ladder_error: None,
        second_look_failures,
    };
    if let Some(r) = stored {
        // The reader holds `escalate` and its reasoning exactly when asked.
        if let (Some(escalate), Some(reasoning)) = (r.escalate, r.escalation_reasoning) {
            view.escalation = Some(EscalationView {
                escalate,
                reasoning,
            });
        }
        view.overall = r.overall;
        view.ladder = r.ladder.map(ladder_view);
        view.ladder_error = r.ladder_error;
    }
    Ok(view)
}

/// The stored evidence run `run_id`. Every citation's text is the paper's at
/// its span, and the spans are in the run's own text's UTF-16 units. It is
/// read in one transaction, so a step saved meanwhile (a run under way) is
/// seen whole or not at all, never as a half-written run.
///
/// # Errors
///
/// [`StoreError::NotFound`] when there is no such run; a row the readers
/// refuse, or a span outside the run's text, as [`StoreError::Corrupt`]; any
/// other store failure as [`StoreError`].
pub fn evidence_run_view(db: &mut dyn Db, run_id: i64) -> Result<EvidenceRunView, StoreError> {
    transaction_with(db, |tx| read_run_view(tx, run_id))
}

/// [`evidence_run_view`]'s reads, on a transaction it opened.
fn read_run_view(db: &mut dyn Db, run_id: i64) -> Result<EvidenceRunView, StoreError> {
    let run = stored_run(db, run_id)?
        .ok_or_else(|| StoreError::NotFound(format!("evidence run {run_id}")))?;
    let text = RunText {
        run_id,
        canonical_id: run.canonical_id,
        text: &run.text,
        offsets: utf16_offsets(&run.text.text),
    };
    let design = stored_design(db, run_id)?
        .map(|d| design_view(&text, d))
        .transpose()?;
    let outcomes = stored_outcomes(db, run_id)?
        .into_iter()
        .map(|o| outcome_view(db, &text, &run, o))
        .collect::<Result<Vec<_>, _>>()?;
    let d1 = EvidenceStep::Rob2(Rob2Step::Domain {
        domain: Domain::D1,
        outcome: None,
    });
    let study = run
        .steps
        .iter()
        .find(|s| s.step == d1)
        .map(|s| domain_view(db, &text, s, Domain::D1))
        .transpose()?;
    // A step set back to pending keeps its error until it is saved, so one
    // a stopped Continue never reached still says why; a done one holds none.
    let failed_steps = run
        .steps
        .iter()
        .filter_map(|s| {
            s.failure.as_ref().map(|f| NamedErrorView {
                label: step_label(&s.step, outcomes.len()),
                error: f.message.clone(),
            })
        })
        .collect();
    let has_rob2_result = run.steps.iter().any(|s| {
        matches!(s.step, EvidenceStep::Rob2(Rob2Step::Domain { .. }))
            && s.status == StepStatus::Done
    });
    let weighed: Vec<f64> = outcomes
        .iter()
        .filter_map(|o| o.ladder.as_ref().map(|l| l.weight))
        .collect();
    // bmlib's `paper_weight` is the largest ladder weight; taken from the
    // stored weights, so an old run's view needs no rules of today's.
    let finished = run.status == RunStatus::Complete
        && run.kind == Some(RunKind::Assessed)
        && run.steps.iter().all(|s| s.status == StepStatus::Done);
    let paper_weight = if finished {
        weighed.iter().copied().reduce(f64::max)
    } else {
        None
    };
    Ok(EvidenceRunView {
        id: run_id,
        canonical_id: run.canonical_id,
        condensed: run.steps.iter().any(|s| s.condensed == Some(true)),
        model: run.model.clone(),
        created_at: run.created_at.clone(),
        first_shown_at: run.first_shown_at.clone(),
        status: run.status,
        kind: run.kind,
        earlier_rules: run.earlier_rules,
        too_long: run.too_long,
        design,
        assessed_outcomes: weighed.len(),
        outcomes,
        study,
        paper_weight,
        failed_steps,
        has_rob2_result,
    })
}

/// The Evidence weight tab of `paper`: its latest text, the newest run on it
/// and every other run, read in one transaction. A run on an older text is
/// listed in `earlier`, never current. A newest run that reads as
/// [`StoreError::Corrupt`] or a [`DbError::Column`] is logged and held apart
/// in `current_error`.
///
/// # Errors
///
/// [`StoreError::NotFound`] when the paper has no canonical text; any other
/// store failure as [`StoreError`].
pub fn evidence_view(db: &mut dyn Db, paper: &PaperId) -> Result<EvidenceView, StoreError> {
    transaction_with(db, |tx| read_view(tx, paper))
}

/// [`evidence_view`]'s reads, on a transaction it opened.
fn read_view(db: &mut dyn Db, paper: &PaperId) -> Result<EvidenceView, StoreError> {
    let canonical_id = latest_canonical_id(db, paper)?
        .ok_or_else(|| StoreError::NotFound(format!("full text of paper {}", paper.as_str())))?;
    let current_id = latest_run(db, paper, canonical_id)?;
    let held_apart = |why: String| {
        log::error!(
            "paper {}: its newest evidence run cannot be read: {why}",
            paper.as_str()
        );
        (None, Some(why))
    };
    let (current, current_error) = match current_id.map(|id| read_run_view(db, id)).transpose() {
        Ok(current) => (current, None),
        Err(StoreError::Corrupt(why)) => held_apart(why),
        // A value of the wrong type is damage too: SQLite does not enforce
        // column types. A failing database (`Backend`) still fails the view.
        Err(StoreError::Db(e @ DbError::Column(_))) => held_apart(e.to_string()),
        Err(e) => return Err(e),
    };
    let earlier = runs(db, paper)?
        .into_iter()
        .filter(|r| Some(r.id) != current_id)
        .collect();
    Ok(EvidenceView {
        canonical_id,
        current,
        current_error,
        earlier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence_store::tests::{
        asked, assessed_run, d5, domain_step, escalation_step, outcome, outcome_name, save_pending,
        save_step, step_id, weighed,
    };
    use crate::evidence_store::{
        create_run, fail_step, finish_run, pending_steps, save_design, save_domain,
        save_escalation, StepFailure,
    };
    use crate::identifiers::Identifiers;
    use crate::intake::SourceKind;
    use crate::pico_store::tests::{cite, paper_with_text};
    use crate::store::{
        canonical_by_id, create_paper, open_memory_store, save_canonical, save_source, NewSource,
        SourcePlace,
    };
    use bmlib::db::{execute, Value};
    use bmlib::fulltext::canonical::{assess_full_text, canonical_from_markdown};
    use bmlib::quality::{
        build_ladder, paper_weight, DesignProposal, Judgement, LadderSettings, Level, PicoCitation,
        RctLayout, StudyDesign, DESIGN_PROMPT_VERSION,
    };
    use rusqlite::Connection;

    /// A paper with astral characters before the cited block.
    const ASTRAL: &str = "# Trial 😀\n\n## Méthods 𝛼\n\nBody 😀 text.\n\nAfter.\n";

    /// The cited block of [`ASTRAL`].
    const NEEDLE: &str = "Body 😀 text.";

    /// The view of run `run_id`.
    fn view(c: &mut Connection, run_id: i64) -> EvidenceRunView {
        evidence_run_view(c, run_id).unwrap()
    }

    /// Stores `md` as a new canonical text of `paper`.
    fn add_text(c: &mut Connection, paper: &PaperId, md: &str) -> i64 {
        let sid = save_source(
            c,
            paper,
            NewSource {
                kind: SourceKind::Markdown,
                origin: "paste".into(),
                file_name: None,
                bytes: md.as_bytes().to_vec(),
                decoded: None,
            },
            SourcePlace::App,
        )
        .unwrap();
        let text = canonical_from_markdown(md);
        let a = assess_full_text(&text).unwrap();
        save_canonical(c, paper, sid, &text, &a).unwrap()
    }

    /// A paper on [`ASTRAL`] with a run whose design cites [`NEEDLE`], by
    /// code point; returns the paper, its text and the run.
    fn astral_run(c: &mut Connection) -> (PaperId, i64, i64) {
        let p = create_paper(c, &Identifiers::default(), Some("Trial")).unwrap();
        let cid = add_text(c, &p, ASTRAL);
        let t = canonical_by_id(c, &p, cid).unwrap().unwrap().text;
        let byte = t.text.find(NEEDLE).unwrap();
        let start = t.text[..byte].chars().count();
        let run_id = create_run(c, &p, cid, "m").unwrap();
        let step = pending_steps(c, run_id).unwrap()[0].id;
        let proposal = DesignProposal {
            design: StudyDesign::Rct,
            rct_layout: Some(RctLayout::Parallel),
            citations: vec![PicoCitation {
                start,
                end: start + NEEDLE.chars().count(),
                text: NEEDLE.into(),
                similarity: 0.9,
                exact: false,
            }],
            quotes_not_found: vec![],
            contradicted: vec![],
            quotes_dropped: 0,
            reasoning: "Randomised.".into(),
            condensed: false,
            prompt_version: DESIGN_PROMPT_VERSION,
        };
        save_design(c, step, &proposal).unwrap();
        (p, cid, run_id)
    }

    /// The paper text `c` slices out of canonical text `cid`, in UTF-16 units.
    fn sliced(c: &mut Connection, p: &PaperId, cid: i64, cited: &PrismaCitationView) -> String {
        let t = canonical_by_id(c, p, cid).unwrap().unwrap().text;
        let units: Vec<u16> = t.text.encode_utf16().collect();
        String::from_utf16(&units[cited.span.start..cited.span.end]).unwrap()
    }

    /// With RoB 2's plan pending, both outcomes show D2–D5 and the trial D1,
    /// every one pending with no judgement, no number and no paper weight;
    /// the design and outcomes show as saved.
    #[test]
    fn pending_domains_are_not_assessed() {
        let mut c = open_memory_store().unwrap();
        let (_, _, run_id) = assessed_run(&mut c);
        let v = view(&mut c, run_id);
        assert_eq!(v.kind, Some(RunKind::Assessed));
        assert_eq!(v.status, RunStatus::Incomplete);
        assert!(!v.earlier_rules);
        assert_eq!(v.outcomes.len(), 2);
        for (i, o) in v.outcomes.iter().enumerate() {
            assert_eq!(o.index, i);
            let names: Vec<&str> = o.domains.iter().map(|d| d.domain.as_str()).collect();
            assert_eq!(names, ["D2", "D3", "D4", "D5"]);
            for d in &o.domains {
                assert_eq!((d.status, &d.judgement), (StepStatus::Pending, &None));
                assert!(d.answers.is_empty() && d.facts.is_none() && d.analysis.is_none());
            }
            assert_eq!(
                (&o.overall, &o.ladder, &o.escalation),
                (&None, &None, &None)
            );
        }
        let study = v.study.unwrap();
        assert_eq!(study.domain, "D1");
        assert_eq!(study.name, domain_title(Domain::D1));
        assert_eq!((study.status, study.judgement), (StepStatus::Pending, None));
        assert_eq!((v.assessed_outcomes, v.paper_weight), (0, None));
        assert!(!v.has_rob2_result);
        assert!(v.failed_steps.is_empty());
        let design = v.design.unwrap();
        assert_eq!(design.design, "rct");
        assert_eq!(design.rct_layout.as_deref(), Some("parallel"));
        assert_eq!(design.quotes.citations[0].text, "Para one.");
        assert_eq!(design.quotes.quotes_not_found, ["Nowhere."]);
        assert_eq!(design.quotes.quotes_contradicted[0].contradiction, "number");
        assert_eq!(design.quotes.quotes_dropped, 1);
        let o = &v.outcomes[0];
        assert_eq!(o.name, outcome_name(0));
        assert_eq!(o.timepoint.as_deref(), Some("12 months"));
        let r = o.result.as_ref().unwrap();
        assert_eq!(
            (r.measure.as_str(), r.scale.as_str()),
            ("mean difference", "difference")
        );
        assert_eq!(
            (r.estimate, r.ci_low, r.n_analysed),
            (-0.4, Some(-0.7), Some(312))
        );
        assert_eq!(o.quotes.citations[0].text, "Para two.");
    }

    /// A failed domain shows its error and no judgement; its outcome has no
    /// number, and a failed escalation names its error and the ladder's.
    /// Both are listed by their progress labels.
    #[test]
    fn a_failed_domain_shows_its_error_and_no_number() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let d3 = domain_step(Domain::D3, Some(0));
        let esc = escalation_step(0);
        save_pending(&mut c, &p, cid, run_id, &[d3, esc]);
        let failure = StepFailure {
            message: "The reply was not JSON.".into(),
            retryable: false,
            status: None,
        };
        let id = step_id(&mut c, run_id, d3);
        fail_step(&mut c, id, &failure).unwrap();
        let failed = bmlib::quality::Rob2Escalation::Failed {
            reason: "The server is busy.".into(),
            retryable: true,
            status: Some(503),
        };
        let o0 = outcome(&mut c, &p, cid, 0);
        let id = step_id(&mut c, run_id, esc);
        save_escalation(&mut c, id, &failed, &weighed(&o0, &failed)).unwrap();
        let v = view(&mut c, run_id);
        let o = &v.outcomes[0];
        let d = &o.domains[1];
        assert_eq!(d.domain, "D3");
        assert_eq!(d.status, StepStatus::Failed);
        assert_eq!(d.error.as_deref(), Some("The reply was not JSON."));
        assert_eq!(d.judgement, None);
        assert!(d.answers.is_empty());
        assert_eq!((&o.overall, &o.ladder), (&None, &None));
        assert_eq!(o.escalation_error.as_deref(), Some("The server is busy."));
        assert_eq!(o.ladder_error.as_deref(), Some("the escalation failed"));
        assert!(v.outcomes[1].ladder.is_some());
        let named = |label: &str, error: &str| NamedErrorView {
            label: label.into(),
            error: error.into(),
        };
        assert_eq!(
            v.failed_steps,
            [
                named(
                    "Outcome 1 of 2: D3, missing outcome data",
                    "The reply was not JSON."
                ),
                named(
                    "Outcome 1 of 2: the overall judgement",
                    "The server is busy."
                ),
            ]
        );
        assert_eq!((v.assessed_outcomes, v.paper_weight), (1, None));
    }

    /// An incomplete run has no paper weight, though one outcome has a
    /// number; complete, it is bmlib's `paper_weight` of the run's ladders.
    #[test]
    fn paper_weight_only_when_complete() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let esc1 = escalation_step(1);
        save_pending(&mut c, &p, cid, run_id, &[esc1]);
        let v = view(&mut c, run_id);
        assert_eq!(v.status, RunStatus::Incomplete);
        assert_eq!((v.assessed_outcomes, v.outcomes.len()), (1, 2));
        assert_eq!(v.paper_weight, None);
        let o1 = outcome(&mut c, &p, cid, 1);
        let mut low = weighed(&o1, &asked());
        low.rob2.overall = Some(Judgement::Low);
        low.ladder = Some(
            build_ladder(
                Level::High,
                Judgement::Low,
                o1.result.as_ref(),
                &LadderSettings::default(),
            )
            .unwrap(),
        );
        let id = step_id(&mut c, run_id, esc1);
        save_escalation(&mut c, id, &asked(), &low).unwrap();
        assert!(finish_run(&mut c, run_id).unwrap());
        let o0 = outcome(&mut c, &p, cid, 0);
        let ladders = [
            weighed(&o0, &asked()).ladder.unwrap(),
            low.ladder.clone().unwrap(),
        ];
        assert_ne!(ladders[0].weight, ladders[1].weight, "a max worth checking");
        let v = view(&mut c, run_id);
        assert_eq!(v.status, RunStatus::Complete);
        assert_eq!(v.assessed_outcomes, 2);
        assert!(v.paper_weight.is_some());
        assert_eq!(v.paper_weight, paper_weight(&ladders));
        let ladder = v.outcomes[0].ladder.as_ref().unwrap();
        assert_eq!(ladder.weight, ladders[0].weight);
        assert_eq!(ladder.steps[0].reason, "risk_of_bias");
        assert_eq!(ladder.steps[0].judgement.as_deref(), Some("some_concerns"));
        let e = v.outcomes[0].escalation.as_ref().unwrap();
        assert_eq!((e.escalate, e.reasoning.as_str()), (false, "Not serious."));
    }

    /// A D5 whose second look failed keeps its outcome's number, with the
    /// failure named beside it; D5 shows its facts (with their meaning),
    /// measurements and analysis check.
    #[test]
    fn a_failed_second_look_is_named_beside_the_number() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        let d5_0 = domain_step(Domain::D5, Some(0));
        save_pending(&mut c, &p, cid, run_id, &[d5_0]);
        let q = cite(&mut c, &p, cid, "Para one.");
        let id = step_id(&mut c, run_id, d5_0);
        save_domain(&mut c, id, &d5(&outcome_name(0), Some(q.clone()), Some(q))).unwrap();
        let v = view(&mut c, run_id);
        let o = &v.outcomes[0];
        assert!(o.ladder.is_some());
        assert_eq!(
            o.second_look_failures,
            [NamedErrorView {
                label: "at 6 months".into(),
                error: "the server is busy".into(),
            }]
        );
        assert!(v.outcomes[1].second_look_failures.is_empty());
        let d = &o.domains[3];
        assert_eq!((d.domain.as_str(), d.status), ("D5", StepStatus::Done));
        let facts = d.facts.as_ref().unwrap();
        assert_eq!(facts.len(), 8);
        let fixed = facts.iter().find(|f| f.fact == "measure_fixed").unwrap();
        assert!(fixed.claimed);
        assert_eq!(fixed.meaning, fact_meaning(D5Fact::MeasureFixed));
        assert_eq!(fixed.quotes.citations[0].text, "Para one.");
        let [failed, looked] = &d.measurements[..] else {
            panic!("not two measurements");
        };
        assert_eq!(failed.second_look, None);
        assert_eq!(
            failed.second_look_error.as_deref(),
            Some("the server is busy")
        );
        assert_eq!(failed.named[0].text, "Para one.");
        assert_eq!(failed.quotes.quotes_not_found, ["Gone."]);
        assert_eq!(looked.second_look, Some(vec![]));
        assert!(!d.analysis.as_ref().unwrap().differs);
        assert_eq!(v.paper_weight, None);
        assert!(v.failed_steps.is_empty());
    }

    /// A run under earlier rules is flagged and still shows every domain; a
    /// question today's bmlib does not ask is shown by its id.
    #[test]
    fn earlier_rules_are_flagged() {
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
            "UPDATE evidence_runs SET rob2_prompt_version = rob2_prompt_version + 1 WHERE id = ?",
            &[Value::from(run_id)],
        )
        .unwrap();
        let v = view(&mut c, run_id);
        assert!(v.earlier_rules);
        let study = v.study.unwrap();
        assert!(study.judgement.is_some());
        let unknown = study
            .answers
            .iter()
            .find(|a| a.question_id == "9.9")
            .unwrap();
        assert_eq!(unknown.question, "9.9");
        let known = study
            .answers
            .iter()
            .find(|a| a.question_id != "9.9")
            .unwrap();
        assert_eq!(known.question, question_text(&known.question_id));
        assert!(v
            .outcomes
            .iter()
            .all(|o| o.domains.iter().all(|d| d.judgement.is_some())));
    }

    /// A run says it was condensed once any step was.
    #[test]
    fn condensed_run_says_so() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let run_id = create_run(&mut c, &p, cid, "m").unwrap();
        let design = pending_steps(&mut c, run_id).unwrap().remove(0);
        save_step(&mut c, &p, cid, &design);
        let v = view(&mut c, run_id);
        assert!(!v.condensed, "the design was not condensed");
        assert!(v.design.is_some() && v.outcomes.is_empty() && v.study.is_none());
        assert_eq!(v.kind, None);
        let outcomes = pending_steps(&mut c, run_id).unwrap().remove(0);
        save_step(&mut c, &p, cid, &outcomes);
        assert!(view(&mut c, run_id).condensed, "the outcomes were");
    }

    /// A citation after an astral character maps to UTF-16 units that slice
    /// the paper to exactly its text.
    #[test]
    fn spans_are_utf16() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = astral_run(&mut c);
        let cited = view(&mut c, run_id)
            .design
            .unwrap()
            .quotes
            .citations
            .remove(0);
        assert_eq!(cited.text, NEEDLE);
        assert_eq!(sliced(&mut c, &p, cid, &cited), NEEDLE);
        assert!(
            cited.span.end - cited.span.start > NEEDLE.chars().count(),
            "astral units counted twice"
        );
        assert_eq!((cited.similarity, cited.exact), (0.9, false));
    }

    /// After a new text, the old run is listed as earlier and still computes
    /// its spans against its own text.
    #[test]
    fn earlier_run_uses_its_text() {
        let mut c = open_memory_store().unwrap();
        let (p, old, run_id) = astral_run(&mut c);
        let new = add_text(&mut c, &p, "# T\n\nAnother text.\n");
        let v = evidence_view(&mut c, &p).unwrap();
        assert_eq!(v.canonical_id, new);
        assert_eq!(v.current, None);
        assert_eq!(v.earlier.iter().map(|r| r.id).collect::<Vec<_>>(), [run_id]);
        let alone = view(&mut c, run_id);
        assert_eq!(alone.canonical_id, old);
        let cited = &alone.design.unwrap().quotes.citations[0];
        assert_eq!(sliced(&mut c, &p, old, cited), NEEDLE);
        let second = create_run(&mut c, &p, new, "m").unwrap();
        let v = evidence_view(&mut c, &p).unwrap();
        assert_eq!(v.current.unwrap().id, second);
        assert_eq!(v.earlier.len(), 1);
    }

    /// A newest run that reads as `Corrupt` is held apart in `current_error`,
    /// and the earlier runs are still listed (#165).
    #[test]
    fn a_corrupt_current_run_is_held_apart() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let first = create_run(&mut c, &p, cid, "m").unwrap();
        let second = create_run(&mut c, &p, cid, "m").unwrap();
        // A kind its single design step cannot have: not the plan.
        bmlib::db::execute(
            &mut c,
            "UPDATE evidence_runs SET kind = 'assessed' WHERE id = ?",
            &[bmlib::db::Value::from(second)],
        )
        .unwrap();
        let v = evidence_view(&mut c, &p).unwrap();
        assert_eq!(v.current, None);
        let why = v.current_error.unwrap();
        assert!(why.contains(&format!("evidence run {second}")), "{why}");
        assert_eq!(v.earlier.iter().map(|r| r.id).collect::<Vec<_>>(), [first]);
        let third = create_run(&mut c, &p, cid, "m").unwrap();
        let v = evidence_view(&mut c, &p).unwrap();
        assert_eq!((v.current.unwrap().id, v.current_error), (third, None));
    }

    /// A newest run holding a value of the wrong type is held apart too; a
    /// failing database still fails the view (#165).
    #[test]
    fn a_mistyped_current_run_is_held_apart_but_a_db_failure_is_not() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let first = create_run(&mut c, &p, cid, "m").unwrap();
        let second = create_run(&mut c, &p, cid, "m").unwrap();
        execute(
            &mut c,
            "UPDATE evidence_runs SET rob2_prompt_version = 'x' WHERE id = ?",
            &[Value::from(second)],
        )
        .unwrap();
        let v = evidence_view(&mut c, &p).unwrap();
        assert_eq!(v.current, None);
        let why = v.current_error.unwrap();
        assert!(why.contains("rob2_prompt_version"), "{why}");
        assert_eq!(v.earlier.iter().map(|r| r.id).collect::<Vec<_>>(), [first]);
        execute(&mut c, "DROP TABLE evidence_designs", &[]).unwrap();
        create_run(&mut c, &p, cid, "m").unwrap();
        assert!(matches!(
            evidence_view(&mut c, &p),
            Err(StoreError::Db(DbError::Backend(_)))
        ));
    }

    /// A run marked too long for its window says so.
    #[test]
    fn too_long_is_shown() {
        let mut c = open_memory_store().unwrap();
        let (p, cid) = paper_with_text(&mut c);
        let run_id = create_run(&mut c, &p, cid, "m").unwrap();
        assert!(!view(&mut c, run_id).too_long);
        crate::evidence_store::mark_too_long(&mut c, run_id).unwrap();
        assert!(view(&mut c, run_id).too_long);
    }

    /// A RoB 2 result is on show only once a domain step is done: not while
    /// pending, nor after D1 failed.
    #[test]
    fn has_rob2_result_needs_a_done_domain() {
        let mut c = open_memory_store().unwrap();
        let (p, cid, run_id) = assessed_run(&mut c);
        assert!(!view(&mut c, run_id).has_rob2_result);
        let d1 = step_id(&mut c, run_id, domain_step(Domain::D1, None));
        let failure = StepFailure {
            message: "x".into(),
            retryable: false,
            status: None,
        };
        fail_step(&mut c, d1, &failure).unwrap();
        assert!(!view(&mut c, run_id).has_rob2_result);
        let d2 = pending_steps(&mut c, run_id)
            .unwrap()
            .into_iter()
            .find(|s| s.step == domain_step(Domain::D2, Some(0)))
            .unwrap();
        save_step(&mut c, &p, cid, &d2);
        assert!(view(&mut c, run_id).has_rob2_result);
    }

    /// A stored span outside the run's text is `Corrupt` (held apart in the
    /// tab's view), never a panic; an unknown run or paper is not found.
    #[test]
    fn a_span_outside_the_text_is_corrupt() {
        let mut c = open_memory_store().unwrap();
        let (p, _, run_id) = assessed_run(&mut c);
        execute(&mut c, "UPDATE evidence_citations SET span_end = 9999", &[]).unwrap();
        assert!(matches!(
            evidence_run_view(&mut c, run_id),
            Err(StoreError::Corrupt(_))
        ));
        let v = evidence_view(&mut c, &p).unwrap();
        assert!(v.current.is_none() && v.current_error.is_some());
        assert!(matches!(
            evidence_run_view(&mut c, run_id + 1),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            evidence_view(&mut c, &PaperId("nope".into())),
            Err(StoreError::NotFound(_))
        ));
    }

    /// Names today's bmlib does not write, each set on a stored run by SQL
    /// (as an earlier bmlib might have written them), with the step whose
    /// row holds it: a design, a scale, a D5 fact and verdict, a ladder
    /// reason.
    const UNKNOWN_NAMES: [(&str, EvidenceStep); 9] = [
        (
            "UPDATE evidence_designs SET design = 'n_of_1' WHERE step_id = ?",
            EvidenceStep::Design,
        ),
        (
            "UPDATE evidence_outcomes SET scale = 'log_ratio' WHERE step_id = ? AND position = 0",
            EvidenceStep::Outcomes,
        ),
        (
            "UPDATE evidence_d5_facts SET fact = 'plan_public'
             WHERE step_id = ? AND fact = 'plan_cited'",
            EvidenceStep::Rob2(Rob2Step::Domain {
                domain: Domain::D5,
                outcome: Some(0),
            }),
        ),
        (
            "UPDATE evidence_d5_facts SET verdict = 'withdrawn'
             WHERE step_id = ? AND fact = 'switched'",
            EvidenceStep::Rob2(Rob2Step::Domain {
                domain: Domain::D5,
                outcome: Some(0),
            }),
        ),
        (
            "UPDATE evidence_ladder_steps SET reason = 'indirect'
             WHERE step_id = ? AND position = 0",
            EvidenceStep::Rob2(Rob2Step::Escalation { outcome: 0 }),
        ),
        (
            "UPDATE evidence_domains SET judgement = 'unclear' WHERE step_id = ?",
            EvidenceStep::Rob2(Rob2Step::Domain {
                domain: Domain::D1,
                outcome: None,
            }),
        ),
        (
            "UPDATE evidence_answers SET answer = 'maybe' WHERE step_id = ? AND question_id = '1.1'",
            EvidenceStep::Rob2(Rob2Step::Domain {
                domain: Domain::D1,
                outcome: None,
            }),
        ),
        (
            "UPDATE evidence_outcome_results SET overall = 'unclear', ladder_level = 'grade_z'
             WHERE step_id = ?",
            EvidenceStep::Rob2(Rob2Step::Escalation { outcome: 0 }),
        ),
        (
            "UPDATE evidence_ladder_steps SET judgement = 'unclear'
             WHERE step_id = ? AND judgement IS NOT NULL",
            EvidenceStep::Rob2(Rob2Step::Escalation { outcome: 1 }),
        ),
    ];

    /// A whole assessed run on a new paper, with `edits` of
    /// [`UNKNOWN_NAMES`] applied; returns the paper and the run.
    fn run_with_names(c: &mut Connection, edits: &[(&str, EvidenceStep)]) -> (PaperId, i64) {
        let (p, cid, run_id) = assessed_run(c);
        save_pending(c, &p, cid, run_id, &[]);
        for (sql, step) in edits {
            let id = step_id(c, run_id, *step);
            assert_eq!(execute(c, sql, &[Value::from(id)]).unwrap(), 1, "{sql}");
        }
        (p, run_id)
    }

    /// A run under earlier rules is shown in full with names today's bmlib
    /// does not write, each as stored (spec §6: never re-judged); on a run
    /// under the current rules each one alone is `Corrupt`.
    #[test]
    fn an_earlier_run_is_read_without_today_s_names() {
        for edit in &UNKNOWN_NAMES {
            let mut c = open_memory_store().unwrap();
            let (_, run_id) = run_with_names(&mut c, std::slice::from_ref(edit));
            assert!(
                matches!(
                    evidence_run_view(&mut c, run_id),
                    Err(StoreError::Corrupt(_))
                ),
                "{}",
                edit.0
            );
        }
        let mut c = open_memory_store().unwrap();
        let (p, run_id) = run_with_names(&mut c, &UNKNOWN_NAMES);
        execute(
            &mut c,
            "UPDATE evidence_runs SET algorithm_version = 'rob2/0' WHERE id = ?",
            &[Value::from(run_id)],
        )
        .unwrap();
        let v = view(&mut c, run_id);
        assert_eq!(
            evidence_view(&mut c, &p).unwrap().current.as_ref(),
            Some(&v)
        );
        assert!(v.earlier_rules);
        assert_eq!(v.design.as_ref().unwrap().design, "n_of_1");
        let first = &v.outcomes[0];
        assert_eq!(first.result.as_ref().unwrap().scale, "log_ratio");
        let d5 = first.domains.iter().find(|d| d.domain == "D5").unwrap();
        let facts = d5.facts.as_ref().unwrap();
        let fact = facts.iter().find(|f| f.fact == "plan_public").unwrap();
        assert_eq!(fact.meaning, "plan_public");
        let switched = facts.iter().find(|f| f.fact == "switched").unwrap();
        assert_eq!(switched.verdict, "withdrawn");
        let ladder = first.ladder.as_ref().unwrap();
        assert_eq!(ladder.steps[0].reason, "indirect");
        assert_eq!(ladder.level, "grade_z");
        assert_eq!(first.overall.as_deref(), Some("unclear"));
        let study = v.study.as_ref().unwrap();
        assert_eq!(study.judgement.as_deref(), Some("unclear"));
        assert!(study.answers.iter().any(|a| a.answer == "maybe"));
        let second = &v.outcomes[1].ladder.as_ref().unwrap().steps;
        assert!(second
            .iter()
            .any(|s| s.judgement.as_deref() == Some("unclear")));
    }

    /// Only names bmlib knows are put into its words: any other question or
    /// fact is shown as stored.
    #[test]
    fn unknown_names_read_as_stored() {
        assert_eq!(question_of(Domain::D1, "1.1"), question_text("1.1"));
        assert_eq!(question_of(Domain::D1, "2.1"), "2.1", "not D1's");
        assert_eq!(question_of(Domain::D1, "9.9"), "9.9");
        assert_eq!(meaning_of("plan_cited"), fact_meaning(D5Fact::PlanCited));
        assert_eq!(meaning_of("gone"), "gone");
    }
}
