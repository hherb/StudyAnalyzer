//! The reader's verdicts, ladders and conformance (design 2026-11-26, §5,
//! §7), as pure functions over bmlib's public RoB 2 algorithm: no database,
//! no rule of RoB 2 written here.
//!
//! A domain's verdict follows from its answers ([`derived`]); the reader may
//! override it, and the verdict as left ([`effective`]) feeds the outcome's
//! escalation question, overall verdict and ladder ([`outcome_verdict`]).
//! Conformance ([`departures`]) checks each verdict of one rater's record
//! ([`RaterRecord`]) against its own inputs, so one override is one
//! departure, not one at every step after it.

use bmlib::quality::{
    build_ladder, domain_judgement, needs_escalation_decision, overall_judgement, Answers, Domain,
    EvidenceLadder, Judgement, LadderSettings, Level, ReportedResult, Rob2Error,
};

/// The domains judged per outcome, in order: D2 to D5.
pub const OUTCOME_DOMAINS: [Domain; 4] = [Domain::D2, Domain::D3, Domain::D4, Domain::D5];

/// The verdict RoB 2's algorithm gives `domain`'s `answers`, or `None` while
/// a question that applies is unanswered. Answers to questions that do not
/// apply are ignored, as `domain_judgement` ignores them. Any other refusal
/// (a question not of the domain, an answer the question does not allow) is
/// the error: the caller lets no such answer through.
pub fn derived(domain: Domain, answers: &Answers) -> Result<Option<Judgement>, Rob2Error> {
    match domain_judgement(domain, answers) {
        Ok(j) => Ok(Some(j)),
        Err(Rob2Error::MissingAnswer { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// A verdict as the reader leaves it: their override, else the derived one.
pub fn effective(derived: Option<Judgement>, override_: Option<Judgement>) -> Option<Judgement> {
    override_.or(derived)
}

/// One outcome's escalation question, overall verdict and ladder, as
/// [`outcome_verdict`] derives them.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeVerdict {
    /// Whether the escalation question is asked: every domain has a verdict,
    /// none high, and at least two raise some concerns.
    pub needs_escalation: bool,
    /// The overall verdict RoB 2's algorithm gives the domain verdicts and
    /// the escalation answer; `None` while a domain has no verdict or the
    /// escalation asked is unanswered.
    pub derived_overall: Option<Judgement>,
    /// The overall verdict as the reader leaves it ([`effective`]).
    pub overall: Option<Judgement>,
    /// The ladder from that overall verdict, once there is one.
    pub ladder: Option<EvidenceLadder>,
    /// Why bmlib built no ladder from an overall verdict.
    pub ladder_error: Option<String>,
}

/// The five verdicts, if every one is known.
fn all_known(domains: [Option<Judgement>; 5]) -> Option<[Judgement; 5]> {
    let [d1, d2, d3, d4, d5] = domains;
    Some([d1?, d2?, d3?, d4?, d5?])
}

/// One outcome's verdict from its five domain verdicts as the reader left
/// them (`domains`, D1 first: each [`effective`]), the `escalate` answer, the
/// reader's `overall_override`, the ladder's `start` level and the confirmed
/// `result`. The ladder uses `LadderSettings::default()`. An escalation
/// answer given when the question is not asked is kept and not counted.
pub fn outcome_verdict(
    domains: [Option<Judgement>; 5],
    escalate: Option<bool>,
    overall_override: Option<Judgement>,
    start: Level,
    result: Option<&ReportedResult>,
) -> OutcomeVerdict {
    let known = all_known(domains);
    let needs_escalation = known.is_some_and(|d| needs_escalation_decision(&d));
    // Only `EscalationUndecided` is refused: the overall waits for the answer.
    let derived_overall = known.and_then(|d| overall_judgement(&d, escalate).ok());
    let overall = effective(derived_overall, overall_override);
    let (ladder, ladder_error) =
        match overall.map(|j| build_ladder(start, j, result, &LadderSettings::default())) {
            None => (None, None),
            Some(Ok(l)) => (Some(l), None),
            Some(Err(why)) => (None, Some(why)),
        };
    OutcomeVerdict {
        needs_escalation,
        derived_overall,
        overall,
        ladder,
        ladder_error,
    }
}

/// One domain of one rater's record: its answers and its verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct RaterDomain {
    /// The domain.
    pub domain: Domain,
    /// The outcome's index, `None` for D1.
    pub outcome: Option<usize>,
    /// The answers that count, by question id (a question that does not
    /// apply may be among them; RoB 2's algorithm ignores it).
    pub answers: Answers,
    /// The verdict as the rater left it (the reader's: [`effective`]).
    pub verdict: Option<Judgement>,
}

/// One outcome's overall verdict in one rater's record.
#[derive(Debug, Clone, PartialEq)]
pub struct RaterOverall {
    /// The outcome's index.
    pub outcome: usize,
    /// The answer to the escalation question, if given.
    pub escalate: Option<bool>,
    /// The overall verdict as the rater left it.
    pub overall: Option<Judgement>,
}

/// One rater's record: a reader's stored version
/// (`appraisal_store::rater_record`) or a machine run's stored rows
/// (`evidence_rows::rater_record`), in one shape so both are checked alike.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RaterRecord {
    /// Its domains; a domain with nothing recorded may be missing.
    pub domains: Vec<RaterDomain>,
    /// Its overall verdicts; an outcome with nothing recorded may be missing.
    pub overalls: Vec<RaterOverall>,
}

impl RaterRecord {
    /// The record's `domain` for `outcome` (`None`: D1), if it has one.
    pub fn domain(&self, domain: Domain, outcome: Option<usize>) -> Option<&RaterDomain> {
        self.domains
            .iter()
            .find(|d| d.domain == domain && d.outcome == outcome)
    }

    /// The record's overall verdict row for `outcome`, if it has one.
    pub fn overall(&self, outcome: usize) -> Option<&RaterOverall> {
        self.overalls.iter().find(|o| o.outcome == outcome)
    }

    /// Outcome `outcome`'s five domain verdicts as recorded: D1's (the
    /// study's), then its own D2 to D5.
    pub fn outcome_domains(&self, outcome: usize) -> [Option<Judgement>; 5] {
        let verdict = |domain, outcome| self.domain(domain, outcome).and_then(|d| d.verdict);
        let [d2, d3, d4, d5] = OUTCOME_DOMAINS.map(|d| verdict(d, Some(outcome)));
        [verdict(Domain::D1, None), d2, d3, d4, d5]
    }
}

/// Whether record `r` of an appraisal with `outcomes` outcomes is complete:
/// every question that applies answered in D1 and in each outcome's D2 to D5,
/// and the escalation question answered wherever the verdicts as recorded
/// ask it.
pub fn complete(r: &RaterRecord, outcomes: usize) -> bool {
    let empty = Answers::new();
    let answered = |domain, outcome| {
        let answers = r.domain(domain, outcome).map_or(&empty, |d| &d.answers);
        matches!(derived(domain, answers), Ok(Some(_)))
    };
    answered(Domain::D1, None)
        && (0..outcomes).all(|o| {
            OUTCOME_DOMAINS.iter().all(|d| answered(*d, Some(o)))
                && !(all_known(r.outcome_domains(o)).is_some_and(|d| needs_escalation_decision(&d))
                    && r.overall(o).and_then(|v| v.escalate).is_none())
        })
}

/// Where a departure is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepartureScope {
    /// A domain's verdict, with its outcome (`None`: D1).
    Domain(Domain, Option<usize>),
    /// An outcome's overall verdict.
    Overall(usize),
}

/// A verdict that does not follow from its own inputs under RoB 2's rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Departure {
    /// Which verdict.
    pub scope: DepartureScope,
    /// The verdict the rater gave.
    pub given: Judgement,
    /// The verdict the rules give its inputs.
    pub rules: Judgement,
}

/// Every departure in record `r`, domains first: a domain whose verdict
/// differs from what `domain_judgement` gives its answers, and an overall
/// verdict that differs from what `overall_judgement` gives that outcome's
/// domain verdicts as recorded (D1 and its D2 to D5) and its escalation
/// answer. Where the rules give no verdict (an answer missing, a domain
/// verdict missing, an escalation undecided, or answers the rules refuse),
/// nothing departs from them.
pub fn departures(r: &RaterRecord) -> Vec<Departure> {
    let domains = r.domains.iter().filter_map(|d| {
        let given = d.verdict?;
        let rules = derived(d.domain, &d.answers).ok()??;
        (given != rules).then_some(Departure {
            scope: DepartureScope::Domain(d.domain, d.outcome),
            given,
            rules,
        })
    });
    let overalls = r.overalls.iter().filter_map(|o| {
        let given = o.overall?;
        let known = all_known(r.outcome_domains(o.outcome))?;
        let rules = overall_judgement(&known, o.escalate).ok()?;
        (given != rules).then_some(Departure {
            scope: DepartureScope::Overall(o.outcome),
            given,
            rules,
        })
    });
    domains.chain(overalls).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmlib::quality::{Answer, LadderReason};
    use serde_json::Value;

    /// bmlib's shared RoB 2 contract (`tests/fixtures/README.md` names its rev).
    const FIXTURE: &str = include_str!("../tests/fixtures/rob2_algorithm.json");

    /// `name` read as bmlib's serde names it.
    fn named<T: serde::de::DeserializeOwned>(name: &Value) -> T {
        serde_json::from_value(name.clone()).unwrap()
    }

    /// A case's domain and answers.
    fn case(c: &Value) -> (Domain, Answers) {
        let domain = named(&c["domain"]);
        let answers = c["answers"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(q, a)| (q.clone(), named(a)))
            .collect();
        (domain, answers)
    }

    /// Answers from `(question, answer)` pairs.
    fn answers(pairs: &[(&str, Answer)]) -> Answers {
        pairs.iter().map(|(q, a)| (q.to_string(), *a)).collect()
    }

    /// Every case of bmlib's fixture: each official and exhaustive case
    /// derives its expected verdict, a missing answer derives none, and any
    /// other refusal is the error.
    #[test]
    fn derived_follows_bmlib() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let mut cases = 0;
        for key in ["official", "exhaustive"] {
            for c in fixture[key].as_array().unwrap() {
                let (domain, given) = case(c);
                let want: Judgement = named(&c["expected"]);
                assert_eq!(derived(domain, &given), Ok(Some(want)), "{c}");
                cases += 1;
            }
        }
        assert_eq!(cases, 39 + 647);
        for c in fixture["refusals"].as_array().unwrap() {
            let (domain, given) = case(c);
            let got = derived(domain, &given);
            match c["kind"].as_str().unwrap() {
                "MissingAnswer" => assert_eq!(got, Ok(None), "{c}"),
                "NotAllowed" => assert!(matches!(got, Err(Rob2Error::NotAllowed { .. })), "{c}"),
                "UnknownQuestion" => {
                    assert!(matches!(got, Err(Rob2Error::UnknownQuestion { .. })), "{c}")
                }
                other => panic!("an unexpected kind {other}"),
            }
        }
    }

    /// The override wins; without one the derived verdict stands.
    #[test]
    fn effective_is_the_override_else_the_derived() {
        let (low, high) = (Some(Judgement::Low), Some(Judgement::High));
        assert_eq!(effective(low, high), high);
        assert_eq!(effective(low, None), low);
        assert_eq!(effective(None, high), high);
        assert_eq!(effective(None, None), None);
    }

    /// The escalation question is asked only when no domain is high and two
    /// raise some concerns; then the overall waits for its answer, and a
    /// given answer decides it. An answer given when not asked changes
    /// nothing, and a domain with no verdict asks nothing.
    #[test]
    fn the_escalation_is_asked_only_when_needed() {
        let (l, s, h) = (
            Some(Judgement::Low),
            Some(Judgement::SomeConcerns),
            Some(Judgement::High),
        );
        let verdict =
            |domains, escalate| outcome_verdict(domains, escalate, None, Level::High, None);
        let two = verdict([s, s, l, l, l], None);
        assert!(two.needs_escalation);
        assert_eq!((two.derived_overall, two.overall), (None, None));
        assert!(two.ladder.is_none() && two.ladder_error.is_none());
        assert_eq!(verdict([s, s, l, l, l], Some(true)).overall, h);
        assert_eq!(verdict([s, s, l, l, l], Some(false)).overall, s);
        for (domains, want) in [([s, l, l, l, l], s), ([s, s, h, l, l], h), ([l; 5], l)] {
            let v = verdict(domains, Some(true));
            assert!(!v.needs_escalation, "{domains:?}");
            assert_eq!(v.overall, want, "{domains:?}");
        }
        let open = verdict([s, s, None, l, l], None);
        assert!(!open.needs_escalation);
        assert_eq!(open.overall, None);
    }

    /// The overall verdict as left (an override over the derived one) is
    /// what the ladder starts from.
    #[test]
    fn the_ladder_follows_the_overall_as_left() {
        let low = Some(Judgement::Low);
        let v = outcome_verdict([low; 5], None, Some(Judgement::High), Level::High, None);
        assert_eq!(v.derived_overall, low);
        assert_eq!(v.overall, Some(Judgement::High));
        let ladder = v.ladder.unwrap();
        assert_eq!(ladder.start, Level::High);
        assert_eq!(
            ladder.steps[0].reason,
            LadderReason::RiskOfBias(Judgement::High)
        );
        assert_eq!(ladder.steps[0].levels, 2);
    }

    /// A low record: D1 and one outcome's D2 to D5 answered so each is low.
    fn low_record() -> RaterRecord {
        use Answer::{N, Y};
        let given = [
            (
                Domain::D1,
                None,
                answers(&[("1.1", Y), ("1.2", Y), ("1.3", N)]),
            ),
            (
                Domain::D2,
                Some(0),
                answers(&[("2.1", N), ("2.2", N), ("2.6", Y)]),
            ),
            (Domain::D3, Some(0), answers(&[("3.1", Y)])),
            (
                Domain::D4,
                Some(0),
                answers(&[("4.1", N), ("4.2", N), ("4.3", N)]),
            ),
            (
                Domain::D5,
                Some(0),
                answers(&[("5.1", Y), ("5.2", N), ("5.3", N)]),
            ),
        ];
        RaterRecord {
            domains: given
                .into_iter()
                .map(|(domain, outcome, answers)| RaterDomain {
                    domain,
                    outcome,
                    answers,
                    verdict: Some(Judgement::Low),
                })
                .collect(),
            overalls: vec![RaterOverall {
                outcome: 0,
                escalate: None,
                overall: Some(Judgement::Low),
            }],
        }
    }

    /// Complete once every question that applies is answered in every
    /// domain of every outcome; a missing answer, domain or outcome, or an
    /// escalation the verdicts ask and nobody answered, is not complete.
    #[test]
    fn complete_needs_every_applicable_answer_and_escalation() {
        let r = low_record();
        assert!(complete(&r, 1));
        assert!(!complete(&r, 2));
        let mut gap = r.clone();
        gap.domains[3].answers.remove("4.3");
        assert!(!complete(&gap, 1));
        let mut no_d1 = r.clone();
        no_d1.domains.remove(0);
        assert!(!complete(&no_d1, 1));
        let mut ask = r.clone();
        ask.domains[0].verdict = Some(Judgement::SomeConcerns);
        ask.domains[1].verdict = Some(Judgement::SomeConcerns);
        assert!(!complete(&ask, 1));
        ask.overalls[0].escalate = Some(false);
        assert!(complete(&ask, 1));
    }

    /// A conforming record has no departure; a domain verdict off its
    /// answers is one, and so is an overall verdict off its domains'.
    #[test]
    fn departures_check_each_verdict_against_its_own_inputs() {
        let r = low_record();
        assert!(departures(&r).is_empty());
        let mut off = r.clone();
        off.domains[2].verdict = Some(Judgement::High);
        off.overalls[0].overall = Some(Judgement::High);
        let d3 = Departure {
            scope: DepartureScope::Domain(Domain::D3, Some(0)),
            given: Judgement::High,
            rules: Judgement::Low,
        };
        assert_eq!(departures(&off), vec![d3]);
        off.overalls[0].overall = Some(Judgement::Low);
        assert_eq!(
            departures(&off),
            vec![
                d3,
                Departure {
                    scope: DepartureScope::Overall(0),
                    given: Judgement::Low,
                    rules: Judgement::High,
                },
            ]
        );
    }
}
