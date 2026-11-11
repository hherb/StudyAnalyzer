"""Checks in code that a found quote can support its fact (arms E and F, applied offline).

A fact counts in arms E and F when the model claims it and one of its quotes is found in the
paper. A quote can be found and still not show the fact: a bare registration number "dating" a
plan, a sentence reporting deaths "showing" a switched outcome, the primary outcome's measure
quoted for a secondary one. Each check here asks of a quote the least its fact needs; a fact none
of whose found quotes passes is treated as absent. Facts without a check pass unchanged.
"""

from __future__ import annotations

import re

#: A date: a year (alone or in an 8-digit date), a month name, or a numeric day/month/year.
_DATE = re.compile(
    r"\b(19|20)\d{2}(\d{4})?\b"
    r"|\b(jan|feb|mar|apr|may|jun|jul|aug|sep|sept|oct|nov|dec)[a-z]*\.?\s"
    r"|\b\d{1,2}[/.\-]\d{1,2}[/.\-]\d{2,4}\b",
    re.I,
)
#: A phrase placing a plan before the data: prospective, or before / prior to a stage of the trial.
_BEFORE = re.compile(
    r"prospective"
    r"|\b(before|prior to)\b[^.]{0,40}\b(enrol|recruit|randomi|inclusion|first (patient|participant)"
    r"|start|unblind|database|lock|analys|data)",
    re.I,
)
#: The language of a change to an outcome.
_CHANGE = re.compile(
    r"\b(chang|differ(s|ed)? from|different from|amend|modif|replac|instead|original|initial|added"
    r"|add\b|remov|dropp|switch"
    r"|deviat|no longer|revis|substitut|redefin|post hoc|post-hoc)",
    re.I,
)
#: A time expression.
_TIME = re.compile(
    r"\d|\b(day|week|month|year|hour|baseline|follow|end of|discharge|visit|post-?operative"
    r"|post-?treatment|admission)",
    re.I,
)
#: Words of an outcome's name that say nothing about which outcome it is.
_GENERIC = {
    "assessed", "using", "measured", "measure", "outcome", "outcomes", "score", "scores", "change",
    "changes", "from", "baseline", "after", "with", "total", "rate", "rates", "number", "percentage",
    "mean", "days", "weeks", "months", "years", "follow", "end", "the", "and", "for", "of", "at",
    "time", "point", "until", "through", "during", "level", "levels", "proportion", "patients",
    "participants", "children", "adults", "between", "groups", "group", "daily", "minutes",
}


#: Language saying a measure, time point or analysis was set in advance, not only described.
_PLANNING = re.compile(
    r"pre-?specif|pre-?defin|\bplanned\b|a priori|protocol|regist|analysis plan"
    r"|\b(primary|secondary|main|principal) (efficacy |safety )?"
    r"(outcome|end ?-?point|objective|analys[ie]s|analysis model)",
    re.I,
)
#: The facts the planning check applies to: what a plan fixes for the outcome.
FIXED_FACTS = ("measure_fixed", "timepoint_fixed", "analysis_fixed")


def states_a_plan(quotes: list[str]) -> bool:
    """Whether any of *quotes* says something was set in advance (prespecified, planned, in the
    protocol or registration, a primary or secondary outcome or analysis)."""
    return any(_PLANNING.search(q) for q in quotes)


def outcome_stems(outcome: str) -> set[str]:
    """The outcome name's distinctive words, cut to their first five letters."""
    words = re.findall(r"[A-Za-z][A-Za-z\-]{3,}", outcome)
    return {w.lower()[:5] for w in words if w.lower() not in _GENERIC}


def names_outcome(quote: str, outcome: str) -> bool:
    """Whether *quote* shares a distinctive word (by five-letter stem) with the outcome's name.
    An outcome name with no distinctive word passes every quote."""
    stems = outcome_stems(outcome)
    if not stems:
        return True
    return bool(stems & {w.lower()[:5] for w in re.findall(r"[A-Za-z][A-Za-z\-]{3,}", quote)})


def supports(fact: str, quote: str, outcome: str) -> bool:
    """Whether *quote* can support *fact* for *outcome*; a fact without a check always can."""
    if fact == "plan_dated_early":
        return bool(_DATE.search(quote) or _BEFORE.search(quote))
    if fact == "switched":
        return bool(_CHANGE.search(quote))
    if fact == "measure_fixed":
        return names_outcome(quote, outcome)
    if fact == "timepoint_fixed":
        return bool(_TIME.search(quote))
    return True


def guarded(
    fact: str, claimed_present: bool, quotes: list[str], outcome: str, need_plan: bool = False
) -> bool:
    """A fact present before the checks stays present when any of its found quotes passes; with
    *need_plan*, a fact of :data:`FIXED_FACTS` also needs one of its quotes to state a plan (the
    planning language may sit in another quote of the same fact than the one naming the
    outcome, as a list's heading does)."""
    if not (claimed_present and any(supports(fact, q, outcome) for q in quotes)):
        return False
    return not (need_plan and fact in FIXED_FACTS) or states_a_plan(quotes)
