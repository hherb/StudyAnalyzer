"""Arm F of the RoB 2 wording smoke test: arm E with "not reported" made checkable.

Version 3 (after version 2's review in the findings): 5.3 follows bmlib's own wording, "the
reported analysis differs from the planned one", so it compares the main analysis the methods
name with the one whose result is given, both quoted, instead of listing unreported extra
analyses; a result passage that points to a figure, table, supplement or appendix counts as
reported, since the text cannot show what those hold; the measurement list is held to the
outcome assessed.

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

import re
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
F_VERSION = 3

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

#: The list asked, with what it holds.
LISTS: dict[str, str] = {
    "other_measurements": (
        "Every OTHER measurement of the SAME outcome as the one assessed (the same thing "
        "measured) that the methods, protocol or registration name: the same thing on another "
        "instrument or scale, at another time point, or defined another way. Not the "
        "measurement assessed itself, not other outcomes, not subgroups, not within-group "
        "comparisons."
    ),
}

#: The analysis check, with what it asks.
MAIN_ANALYSIS = (
    "main_analysis: the analysis whose result the paper gives for the outcome assessed, set "
    "against the main analysis its methods, protocol or analysis plan name for it. planned: "
    "1-2 passages naming the main analysis (the model or test, its covariates, the analysis "
    "population, the handling of missing data). reported: 1-2 passages giving the result with "
    "the analysis it came from. differs: true only when the analysis reported is NOT the one "
    "named as main (another model, other covariates, another population or handling of missing "
    "data) and the paper gives no reason for the change; false when they match, or when either "
    "is not stated."
)

#: A passage that points to where a result is shown outside the running text.
POINTER = re.compile(r"\b(fig(ure)?s?|tables?|supplement\w*|appendix|appendices)\b", re.I)

F_SECOND_LOOK_SYSTEM = """\
You find passages in reports of randomised trials.

CRITICAL RULES:
1. Quote ONLY passages that are ACTUALLY PRESENT in the text, VERBATIM
2. A passage counts only if it gives the result itself, with its numbers, or points to the \
figure, table or supplement that shows it
3. Return ONLY valid JSON, no explanation"""


def versions_prompt(title: str, paper: str, outcome: ProposedOutcome) -> str:
    """The user turn asking for the plan facts and the two lists about *outcome*."""
    listed = "\n".join(f"- {key}: {meaning}" for key, meaning in PLAN_FACTS.items())
    lists = "\n".join(f"- {key}: {meaning}" for key, meaning in LISTS.items())
    shape = ",\n".join(f'    "{key}": {{"present": false, "quotes": []}}' for key in PLAN_FACTS)
    item = (
        '{"label": "short name, e.g. pain VAS at 6 months", "named": ["passage naming it"], '
        '"result": ["passage giving its result, with numbers, or pointing to its figure or table"]}'
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
            "List: for each item give a short label, named (1-2 passages from the methods, "
            "protocol or registration that name it) and result (1-2 passages from the results, a "
            "table or the abstract that give its result with its numbers, or that point to the "
            "figure, table or supplement showing it; an empty list when the paper gives no result "
            "for it, or only a summary without numbers). An empty list when there is no such "
            f"item.\n{lists}",
            f"Analysis check:\n- {MAIN_ANALYSIS}",
            "Respond with JSON in exactly this shape, every key present:\n\n{\n  \"facts\": {\n"
            + shape
            + "\n  },\n"
            + f'  "other_measurements": [{item}],\n'
            + '  "main_analysis": {"differs": false, "planned": ["..."], "reported": ["..."]}\n}',
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
            "result with its numbers, or that point to the figure, table or supplement showing "
            "it. If the paper gives no result for it, or only a summary without numbers and "
            "without such a pointer, answer found: false with no quotes.",
            'Respond with JSON in exactly this shape:\n\n{"found": true, "quotes": ["..."]}',
            f"Paper Title: {title}",
            f"Paper Text:\n{paper}",
        ]
    )


def shows_result(citations: list[PicoCitation]) -> bool:
    """Whether any found passage holds a digit, the least a result with numbers shows, or points
    to a figure, table, supplement or appendix, whose content the text cannot show."""
    return any(any(ch.isdigit() for ch in c.text) or POINTER.search(c.text) for c in citations)


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
        """A found result passage with a number or a pointer, at the first or the second look."""
        return shows_result(self.result) or shows_result(self.second_look or [])

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
class AnalysisCheck:
    """The main analysis named against the one reported, as the paper bears them out."""

    differs: bool
    planned: list[PicoCitation]
    reported: list[PicoCitation]

    @property
    def counts(self) -> bool:
        """Claimed to differ, with a found passage for each side."""
        return self.differs and bool(self.planned) and bool(self.reported)

    def to_dict(self) -> dict[str, Any]:
        """The check for the result file."""
        return {
            "differs": self.differs,
            "planned": [c.text for c in self.planned],
            "reported": [c.text for c in self.reported],
            "counts": self.counts,
        }


def read_analysis_check(raw: object, text: str, similarity: float) -> AnalysisCheck:
    """``main_analysis`` of the reply, quotes verified. Raises ``UnreadableReplyError``."""
    if not isinstance(raw, dict) or not isinstance(raw.get("differs"), bool):
        raise UnreadableReplyError("main_analysis is not an object with differs true or false")
    planned, _nf, _c, _d = verify_quotes(raw.get("planned"), text, similarity)
    reported, _nf, _c, _d = verify_quotes(raw.get("reported"), text, similarity)
    return AnalysisCheck(raw["differs"], planned, reported)


@dataclass
class VersionsReply:
    """The facts, the list and the analysis check, read."""

    facts: dict[str, Fact]
    lists: dict[str, list[Item]] = field(default_factory=dict)
    analysis: AnalysisCheck | None = None


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
    analysis = read_analysis_check(data.get("main_analysis"), text, similarity)
    return VersionsReply(facts, lists, analysis)


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
    missing = [i for i in reply.lists["other_measurements"] if i.not_reported]
    facts["measurement_unreported"] = Fact(bool(missing), [c for i in missing for c in i.named], [], 0)
    # In arm F this fact means "the reported analysis differs from the main one" (bmlib's 5.3
    # PY), not "an analysis is unreported": d5_facts.answers_from reads it under its arm E name.
    check = reply.analysis
    differs = check is not None and check.counts
    facts["analysis_unreported"] = Fact(
        differs, [*check.planned, *check.reported] if differs and check else [], [], 0
    )
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

