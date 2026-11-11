"""Arm F of the RoB 2 wording smoke test: arm E with "not reported" made checkable.

Arm E's two "not reported" facts claimed an absence, which no quote can bear out, and they made
D5 high on the model's unchecked word (findings, arm E). Here the model instead lists every other
version of the outcome the methods, protocol or registration name (another scale, time point or
definition), and every other analysis of it, each with the passage that names it and the passage
that gives its result. In code:

1. An item counts only if its "named" quote is found in the paper.
2. It is reported if a "result" quote is found and holds a number: RoB 2's 5.2 asks about a
   result "missing or only summarised", and a summary without numbers is the latter.
3. An item still without a result gets a second, focused look: one call asking where the paper
   gives that item's result. A found quote with a number there makes it reported after all.
4. Only an item both looks miss is "not reported": a measurement makes 5.2 PY, an analysis 5.3
   PY (arm E's rules, :func:`d5_facts.answers_from`).

The plan facts are arm E's, without its two absence facts.

Experiment, not product code: it reuses private pieces of bmlib's ``Rob2Agent``.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from bmlib.quality._full_text_agent import LABEL_CHARS, UnreadableReplyError
from bmlib.quality.outcome_agent import ProposedOutcome
from bmlib.quality.pico_agent import verify_quotes
from bmlib.quality.pico_models import PicoCitation
from bmlib.quality.rob2_agent import DomainFailed, DomainResult, _CallFailedError, _outcome_block
from bmlib.quality.rob2_algorithm import Domain

from d5_facts import E_SYSTEM_PROMPT, FACTS, Fact, assessment_from, read_facts

#: Bumped whenever the prompts or the rules change, so a stored run says which produced it.
F_VERSION = 2

#: Arm E's absence facts, replaced here by the checked lists.
ABSENCE_FACTS = ("measurement_unreported", "analysis_unreported")

#: The plan facts arm F asks: arm E's without its absence facts, with a switch narrowed to the
#: outcome assessed, and whether a change has a reason that does not depend on the results.
PLAN_FACTS = {k: v for k, v in FACTS.items() if k not in ABSENCE_FACTS} | {
    "switched": (
        "The paper says the outcome assessed ITSELF differs from the registration or protocol: "
        "added, dropped, moved to or from primary, or defined or timed differently. A change to "
        "another outcome does not count."
    ),
    "change_explained": (
        "The paper gives a reason for that change to the outcome assessed that does not depend "
        "on its results (for example a ruling of an ethics committee, or a change made and "
        "stated before unblinding)."
    ),
}

#: The two lists, with what each holds.
LISTS: dict[str, str] = {
    "other_measurements": (
        "Every OTHER measurement of the outcome assessed that the methods, protocol or "
        "registration name: another instrument or scale, another time point, or another "
        "definition of it. Not the measurement assessed itself, and not other outcomes."
    ),
    "other_analyses": (
        "Every OTHER analysis of the outcome assessed that the methods, protocol or analysis "
        "plan name that could give a DIFFERENT estimate of its effect: adjusted or unadjusted, "
        "per-protocol or another analysis population, another way of handling missing data, "
        "another statistical test or model. Not the main analysis itself, and not a "
        "re-expression of the same result (a number needed to treat, a percentage, a relative "
        "instead of an absolute measure of the same counts)."
    ),
}

F_SECOND_LOOK_SYSTEM = """\
You find passages in reports of randomised trials.

CRITICAL RULES:
1. Quote ONLY passages that are ACTUALLY PRESENT in the text, VERBATIM
2. A passage counts only if it gives the result itself, with its numbers
3. Return ONLY valid JSON, no explanation"""


def versions_prompt(title: str, paper: str, outcome: ProposedOutcome) -> str:
    """The user turn asking for the plan facts and the two lists about *outcome*."""
    listed = "\n".join(f"- {key}: {meaning}" for key, meaning in PLAN_FACTS.items())
    lists = "\n".join(f"- {key}: {meaning}" for key, meaning in LISTS.items())
    shape = ",\n".join(f'    "{key}": {{"present": false, "quotes": []}}' for key in PLAN_FACTS)
    item = (
        '{"label": "short name, e.g. pain VAS at 6 months", "named": ["passage naming it"], '
        '"result": ["passage giving its result, with numbers"]}'
    )
    return "\n\n".join(
        [
            "Report the following about the randomised trial in the paper below, for the outcome "
            "named. Copy every quote VERBATIM from the paper, character for character, without "
            "ellipses or joined sentences.",
            _outcome_block(outcome),
            "Facts: for each give present (true or false) and quotes, 1-3 passages that show it, "
            "an empty list when it is absent. Mark a fact present only when the paper itself "
            f"states it.\n{listed}",
            "Lists: for each item give a short label, named (1-2 passages from the methods, "
            "protocol or registration that name it) and result (1-2 passages from the results, a "
            "table or the abstract that give its result with its numbers; an empty list when the "
            "paper gives no result for it, or only a summary without numbers). An empty list "
            f"when there is no such item.\n{lists}",
            "Respond with JSON in exactly this shape, every key present:\n\n{\n  \"facts\": {\n"
            + shape
            + "\n  },\n"
            + f'  "other_measurements": [{item}],\n  "other_analyses": []\n}}',
            f"Paper Title: {title}",
            f"Paper Text:\n{paper}",
        ]
    )


def second_look_prompt(title: str, paper: str, outcome: ProposedOutcome, item: Item) -> str:
    """The user turn asking where the paper gives *item*'s result."""
    named = " / ".join(c.text for c in item.named)
    return "\n\n".join(
        [
            "Find where the paper below gives the result of this measurement or analysis of the "
            f'outcome, which its methods name: "{item.label}", named as: "{named}".',
            _outcome_block(outcome),
            "Quote 1-3 passages, VERBATIM, from the results, a table or the abstract that give its "
            "result with its numbers. If the paper gives no result for it, or only a summary "
            "without numbers, answer found: false with no quotes.",
            'Respond with JSON in exactly this shape:\n\n{"found": true, "quotes": ["..."]}',
            f"Paper Title: {title}",
            f"Paper Text:\n{paper}",
        ]
    )


def has_number(citations: list[PicoCitation]) -> bool:
    """Whether any found passage holds a digit, the least a result with numbers shows."""
    return any(any(ch.isdigit() for ch in c.text) for c in citations)


@dataclass
class Item:
    """One other measurement or analysis the model listed, as the paper bears it out."""

    label: str
    named: list[PicoCitation]
    result: list[PicoCitation]
    #: The second look's found passages, if one was made.
    second_look: list[PicoCitation] | None = None
    second_look_error: str | None = None

    @property
    def counts(self) -> bool:
        """Listed with a passage naming it that is found in the paper."""
        return bool(self.named)

    @property
    def reported(self) -> bool:
        """A found result passage with a number, at the first or the second look."""
        return has_number(self.result) or has_number(self.second_look or [])

    @property
    def not_reported(self) -> bool:
        """Named in the paper, and neither look found its result. A failed second look leaves
        the item undecided, and so not counted as missing."""
        return self.counts and not self.reported and self.second_look is not None

    def to_dict(self) -> dict[str, Any]:
        """The item for the result file."""
        return {
            "label": self.label,
            "named": [c.text for c in self.named],
            "result": [c.text for c in self.result],
            "second_look": None if self.second_look is None else [c.text for c in self.second_look],
            "second_look_error": self.second_look_error,
            "counts": self.counts,
            "reported": self.reported,
            "not_reported": self.not_reported,
        }


def read_items(raw: object, key: str, text: str, similarity: float) -> list[Item]:
    """The list *key* of the reply, every quote verified against *text*.

    Raises:
        UnreadableReplyError: the list is missing or not a list, or an item is not an object or
            has no text label.
    """
    if not isinstance(raw, list):
        raise UnreadableReplyError(f"{key} is missing or not a list")
    items = []
    for n, entry in enumerate(raw):
        if not isinstance(entry, dict) or not isinstance(entry.get("label"), str):
            raise UnreadableReplyError(f"{key}[{n}] is not an object with a label")
        named, _nf, _c, _d = verify_quotes(entry.get("named"), text, similarity)
        result, _nf, _c, _d = verify_quotes(entry.get("result"), text, similarity)
        items.append(Item(entry["label"], named, result))
    return items


@dataclass
class VersionsReply:
    """The facts and the two lists, read."""

    facts: dict[str, Fact]
    lists: dict[str, list[Item]] = field(default_factory=dict)


def read_versions(data: dict[str, Any], text: str, similarity: float) -> VersionsReply:
    """The reply's plan facts and lists, quotes verified. Raises ``UnreadableReplyError``."""
    raw = data.get("facts") if isinstance(data, dict) else None
    if not isinstance(raw, dict):
        raise UnreadableReplyError("facts is not an object")
    # read_facts reads arm E's facts; the absence facts are not asked here, and
    # change_explained is read the same way on its own.
    padded = {**raw, **{k: {"present": False, "quotes": []} for k in ABSENCE_FACTS}}
    facts = read_facts({"facts": padded}, text, similarity)
    explained = read_facts(
        {"facts": {k: raw.get("change_explained") if k == "switched" else {"present": False, "quotes": []}
                   for k in FACTS}},
        text,
        similarity,
    )["switched"]
    facts["change_explained"] = explained
    lists = {key: read_items(data.get(key), key, text, similarity) for key in LISTS}
    return VersionsReply(facts, lists)


def read_second_look(data: dict[str, Any], text: str, similarity: float) -> list[PicoCitation]:
    """The second look's found passages; ``found: false`` is none. Raises ``UnreadableReplyError``."""
    if not isinstance(data, dict) or not isinstance(data.get("found"), bool):
        raise UnreadableReplyError("found is not true or false")
    if not data["found"]:
        return []
    citations, _nf, _c, _d = verify_quotes(data.get("quotes"), text, similarity)
    return citations


def with_absence_from_lists(reply: VersionsReply) -> dict[str, Fact]:
    """Arm E's facts, its absence facts decided by the checked lists, and a switch with a reason
    that does not depend on the results no longer counting as one."""
    facts = {k: f for k, f in reply.facts.items() if k in FACTS}
    if reply.facts["change_explained"].present:
        facts["switched"] = Fact(False, [], [], 0)
    for fact, key in (("measurement_unreported", "other_measurements"),
                      ("analysis_unreported", "other_analyses")):
        missing = [i for i in reply.lists[key] if i.not_reported]
        facts[fact] = Fact(bool(missing), [c for i in missing for c in i.named], [], 0)
    return facts


def assess_d5_by_versions(
    agent: Any,
    title: str,
    stripped: str,
    original: str,
    outcome: ProposedOutcome,
    similarity: float,
) -> tuple[DomainResult, VersionsReply | None]:
    """Ask for the facts and lists, take a second look at each item without a result, and assess
    D5. Returns the assessment (``DomainFailed`` when the first call fails) and the reply."""
    label = f"RoB 2 D5 versions of {title[:LABEL_CHARS]!r}"
    try:
        reply = agent._ask_until_readable(
            E_SYSTEM_PROMPT,
            versions_prompt(title, stripped, outcome),
            lambda data: read_versions(data, original, similarity),
            label,
        )
    except _CallFailedError as exc:
        return DomainFailed(Domain.D5, str(exc), exc.retryable, exc.status), None
    for items in reply.lists.values():
        for item in items:
            if not item.counts or item.reported:
                continue
            try:
                item.second_look = agent._ask_until_readable(
                    F_SECOND_LOOK_SYSTEM,
                    second_look_prompt(title, stripped, outcome, item),
                    lambda data: read_second_look(data, original, similarity),
                    f"{label}, second look at {item.label[:LABEL_CHARS]!r}",
                )
            except _CallFailedError as exc:
                item.second_look_error = str(exc)
    return assessment_from(with_absence_from_lists(reply)), reply

