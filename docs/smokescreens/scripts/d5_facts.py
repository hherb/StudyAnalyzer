"""Arm E of the RoB 2 wording smoke test: D5 from facts, not from the model's verdict.

The model is asked only for facts about the trial's planning, each backed by quotes copied from
the paper. A fact counts only when the model says it is there AND at least one of its quotes is
found in the paper (bmlib's ``verify_quotes``, the same check every answer's quotes get). Code then
turns the facts into answers to 5.1, 5.2 and 5.3 by the evidence ladder of bmlib's prompt
versions 3 and 4, and bmlib's ``domain_judgement`` gives the D5 judgement. The "measured in only
one way" exception is dropped: without a plan such an outcome is NI, the conservative answer.

Experiment, not product code: it reuses private pieces of bmlib's ``Rob2Agent``.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from bmlib.quality._full_text_agent import LABEL_CHARS, UnreadableReplyError
from bmlib.quality.outcome_agent import ProposedOutcome
from bmlib.quality.pico_agent import verify_quotes
from bmlib.quality.pico_models import PicoCitation
from bmlib.quality.rob2_agent import (
    DomainAssessment,
    DomainFailed,
    DomainResult,
    MachineAnswer,
    _CallFailedError,
    _outcome_block,
)
from bmlib.quality.rob2_algorithm import Answer, Domain, domain_judgement

#: Bumped whenever the prompt or the rules change, so a stored run says which produced it.
E_VERSION = 1

#: The facts asked, in order, with what each means. The model answers each with
#: ``{"present": true|false, "quotes": [...]}``.
FACTS: dict[str, str] = {
    "plan_cited": (
        "The paper cites a trial registration (registry and number), a published protocol, or a "
        "statistical analysis plan (by reference or as a supplement)."
    ),
    "plan_dated_early": (
        "The paper places that registration, protocol or plan BEFORE the outcome data could be "
        "seen by group: a registration date before recruitment began or before the trial ended, "
        '"prospectively registered", a protocol published, or an analysis plan finalised, before '
        "unblinding or before the data were analysed."
    ),
    "plan_late": (
        "The paper says the registration or plan was made AFTER recruitment began or after the "
        'data were collected ("retrospectively registered", a registration date after the first '
        "participant was enrolled)."
    ),
    "measure_fixed": (
        "For the outcome assessed, the paper states which measurement was planned in advance: "
        "the instrument or scale and how the outcome is defined, as set in the registration, "
        'protocol or plan, or described as prespecified. A bare "prespecified primary outcome" '
        "that does not name the measurement does not count."
    ),
    "timepoint_fixed": (
        "For the outcome assessed, the paper states the time point (or points) planned in "
        "advance, as set in the registration, protocol or plan, or described as prespecified."
    ),
    "analysis_fixed": (
        "For the outcome assessed, the paper states the analysis planned in advance: the "
        "statistical model or test, and the covariates or the handling of missing data, as set "
        "in the registration, protocol or plan, or described as prespecified."
    ),
    "measurement_unreported": (
        "A measurement of the outcome assessed (another scale, or another time point) that the "
        "methods, protocol or registration name, but whose result the paper does not give in "
        "full."
    ),
    "analysis_unreported": (
        "An analysis of the outcome assessed that the methods, protocol or plan name but whose "
        "result the paper does not give, or a result of it labelled post hoc or exploratory."
    ),
    "switched": (
        "The paper says the primary outcome differs from the registered or protocol primary "
        "outcome (changed, added, dropped or demoted)."
    ),
}

E_SYSTEM_PROMPT = """\
You read reports of randomised trials and report what they say about how the trial was planned.

CRITICAL RULES:
1. Report ONLY what is ACTUALLY PRESENT in the text
2. DO NOT infer, assume or judge: a fact the paper does not state is absent
3. Every fact you mark present needs a quote copied VERBATIM from the paper
4. Return ONLY valid JSON, no explanation"""


def facts_prompt(title: str, paper: str, outcome: ProposedOutcome) -> str:
    """The user turn asking for the facts about *outcome* in *paper*."""
    listed = "\n".join(f"- {key}: {meaning}" for key, meaning in FACTS.items())
    shape = ",\n".join(
        f'    "{key}": {{"present": false, "quotes": []}}' for key in FACTS
    )
    return "\n\n".join(
        [
            "Report the following facts about the randomised trial in the paper below, for the "
            "outcome named. For each, give present (true or false) and quotes: 1-3 passages "
            "copied VERBATIM from the paper that show it, character for character, without "
            "ellipses or joined sentences; an empty list when it is absent. Mark a fact present "
            "only when the paper itself states it.",
            _outcome_block(outcome),
            f"Facts:\n{listed}",
            "Respond with JSON in exactly this shape, every key present:\n\n{\n  \"facts\": {\n"
            + shape
            + "\n  }\n}",
            f"Paper Title: {title}",
            f"Paper Text:\n{paper}",
        ]
    )


@dataclass
class Fact:
    """One fact as the model gave it and as the paper bears it out."""

    claimed: bool
    citations: list[PicoCitation]
    quotes_not_found: list[str]
    contradicted: int

    @property
    def present(self) -> bool:
        """Present only when claimed and at least one quote was found in the paper."""
        return self.claimed and bool(self.citations)

    def to_dict(self) -> dict[str, Any]:
        """The fact for the result file."""
        return {
            "claimed": self.claimed,
            "present": self.present,
            "citations": [c.to_dict() for c in self.citations],
            "quotes_not_found": self.quotes_not_found,
            "contradicted": self.contradicted,
        }


def read_facts(data: dict[str, Any], text: str, similarity: float) -> dict[str, Fact]:
    """The reply's facts, each quote verified against *text*.

    Raises:
        UnreadableReplyError: The reply has no ``facts`` object, a fact is missing or not an
            object, or its ``present`` is not true or false.
    """
    raw = data.get("facts") if isinstance(data, dict) else None
    if not isinstance(raw, dict):
        raise UnreadableReplyError("facts is not an object")
    facts: dict[str, Fact] = {}
    for key in FACTS:
        item = raw.get(key)
        if not isinstance(item, dict):
            raise UnreadableReplyError(f"facts.{key} is missing or not an object")
        claimed = item.get("present")
        if not isinstance(claimed, bool):
            raise UnreadableReplyError(f"facts.{key}.present is not true or false")
        citations, not_found, contradicted, _dropped = verify_quotes(
            item.get("quotes"), text, similarity
        )
        facts[key] = Fact(claimed, citations, not_found, len(contradicted))
    return facts


def answers_from(present: dict[str, bool]) -> dict[str, tuple[Answer, list[str]]]:
    """5.1-5.3 from the facts present, each with the facts that decided it.

    The ladder of bmlib's prompt versions 3 (5.2, 5.3) and 4 (5.1): N (Y for 5.1) needs a cited
    plan dated before the data by group were seen that fixes every choice; the same content on
    the paper's word alone is PN (PY for 5.1); a late plan, or anything less, is NI.
    """
    plan = present["plan_cited"] and present["plan_dated_early"] and not present["plan_late"]
    unplanned_ok = not present["plan_late"]

    def ladder(fixed: list[str], low: Answer, low_word: Answer) -> tuple[Answer, list[str]]:
        if all(present[f] for f in fixed):
            if plan:
                return low, ["plan_cited", "plan_dated_early", *fixed]
            if unplanned_ok:
                return low_word, fixed
        return Answer.NI, [f for f in ("plan_late", *fixed) if present[f]]

    if present["switched"]:
        a51 = (Answer.N, ["switched"])
        a52 = (Answer.Y, ["switched"])
    else:
        a51 = ladder(["measure_fixed", "analysis_fixed"], Answer.Y, Answer.PY)
        a52 = (
            (Answer.PY, ["measurement_unreported"])
            if present["measurement_unreported"]
            else ladder(["measure_fixed", "timepoint_fixed"], Answer.N, Answer.PN)
        )
    a53 = (
        (Answer.PY, ["analysis_unreported"])
        if present["analysis_unreported"]
        else ladder(["analysis_fixed"], Answer.N, Answer.PN)
    )
    return {"5.1": a51, "5.2": a52, "5.3": a53}


def assessment_from(facts: dict[str, Fact]) -> DomainAssessment:
    """D5 assessed from *facts*: answers by :func:`answers_from`, judged by bmlib's algorithm."""
    present = {key: fact.present for key, fact in facts.items()}
    answers: dict[str, MachineAnswer] = {}
    for question, (answer, deciding) in answers_from(present).items():
        citations = [c for key in deciding for c in facts[key].citations]
        reasoning = "; ".join(f"{key}: {'yes' if present[key] else 'no'}" for key in FACTS)
        answers[question] = MachineAnswer(answer, citations, reasoning=f"from facts ({reasoning})")
    judgement = domain_judgement(Domain.D5, {q: a.answer for q, a in answers.items()})
    return DomainAssessment(Domain.D5, answers, judgement)


def assess_d5_by_facts(
    agent: Any,
    title: str,
    stripped: str,
    original: str,
    outcome: ProposedOutcome,
    similarity: float,
) -> tuple[DomainResult, dict[str, Fact] | None]:
    """Ask for the facts with *agent*'s call and retry, and assess D5 from them.

    Returns the assessment (a ``DomainFailed`` when the call fails or the reply stays
    unreadable) and the facts read, ``None`` on failure.
    """
    label = f"RoB 2 D5 facts of {title[:LABEL_CHARS]!r}"
    try:
        facts = agent._ask_until_readable(
            E_SYSTEM_PROMPT,
            facts_prompt(title, stripped, outcome),
            lambda data: read_facts(data, original, similarity),
            label,
        )
    except _CallFailedError as exc:
        return DomainFailed(Domain.D5, str(exc), exc.retryable, exc.status), None
    return assessment_from(facts), facts
