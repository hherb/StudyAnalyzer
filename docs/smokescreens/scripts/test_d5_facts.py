"""Checks of arm E's rules and reply reader. Run: <bmlib venv>/bin/python -m pytest test_d5_facts.py"""

import pytest
from bmlib.quality._full_text_agent import UnreadableReplyError
from bmlib.quality.rob2_algorithm import Answer, Judgement

from d5_facts import FACTS, answers_from, assessment_from, read_facts

PAPER = (
    "The trial was registered at ClinicalTrials.gov (NCT01234567) on 3 March 2015, before "
    "recruitment began. The primary outcome was pain on a 0-10 visual analogue scale at 12 "
    "weeks. Pain was analysed with a linear mixed model adjusted for baseline pain."
)


def present(*keys: str) -> dict[str, bool]:
    """Facts with only *keys* present."""
    return {key: key in keys for key in FACTS}


def labels(p: dict[str, bool]) -> dict[str, Answer]:
    return {q: a for q, (a, _deciding) in answers_from(p).items()}


def test_a_cited_dated_plan_that_fixes_everything_is_the_low_rung():
    p = present("plan_cited", "plan_dated_early", "measure_fixed", "timepoint_fixed", "analysis_fixed")
    assert labels(p) == {"5.1": Answer.Y, "5.2": Answer.N, "5.3": Answer.N}


def test_the_same_content_on_the_papers_word_is_one_rung_up():
    p = present("measure_fixed", "timepoint_fixed", "analysis_fixed")
    assert labels(p) == {"5.1": Answer.PY, "5.2": Answer.PN, "5.3": Answer.PN}
    # A cited plan without a date is still only the paper's word.
    p = present("plan_cited", "measure_fixed", "timepoint_fixed", "analysis_fixed")
    assert labels(p) == {"5.1": Answer.PY, "5.2": Answer.PN, "5.3": Answer.PN}


def test_a_late_plan_or_anything_less_is_ni():
    p = present("plan_cited", "plan_dated_early", "plan_late", "measure_fixed", "timepoint_fixed", "analysis_fixed")
    assert labels(p) == {"5.1": Answer.NI, "5.2": Answer.NI, "5.3": Answer.NI}
    assert labels(present()) == {"5.1": Answer.NI, "5.2": Answer.NI, "5.3": Answer.NI}
    # A plan that leaves the time point open does not reach N on 5.2.
    p = present("plan_cited", "plan_dated_early", "measure_fixed", "analysis_fixed")
    assert labels(p) == {"5.1": Answer.Y, "5.2": Answer.NI, "5.3": Answer.N}


def test_selection_signs_win():
    p = present("plan_cited", "plan_dated_early", "measure_fixed", "timepoint_fixed", "analysis_fixed",
                "measurement_unreported", "analysis_unreported")
    assert labels(p) == {"5.1": Answer.Y, "5.2": Answer.PY, "5.3": Answer.PY}
    assert labels(present("switched")) == {"5.1": Answer.N, "5.2": Answer.Y, "5.3": Answer.NI}


def reply(**claims: list[str]) -> dict:
    """A reply claiming each named fact with its quotes, every other fact absent."""
    return {"facts": {key: {"present": key in claims, "quotes": claims.get(key, [])} for key in FACTS}}


def test_a_fact_counts_only_when_its_quote_is_found():
    data = reply(
        plan_cited=["registered at ClinicalTrials.gov (NCT01234567)"],
        plan_dated_early=["on 3 March 2015, before recruitment began"],
        measure_fixed=["pain on a 0-10 visual analogue scale"],
        timepoint_fixed=["at 12 weeks"],
        analysis_fixed=["The analysis plan was finalised before unblinding."],  # not in the paper
    )
    facts = read_facts(data, PAPER, 0.99)
    assert facts["plan_cited"].present and facts["timepoint_fixed"].present
    assert facts["analysis_fixed"].claimed and not facts["analysis_fixed"].present
    assessment = assessment_from(facts)
    answers = {q: a.answer for q, a in assessment.answers.items()}
    assert answers == {"5.1": Answer.NI, "5.2": Answer.N, "5.3": Answer.NI}
    assert assessment.judgement is Judgement.SOME_CONCERNS
    assert assessment.answers["5.2"].citations, "the deciding facts' quotes are its citations"


def test_an_unreadable_reply_is_refused():
    with pytest.raises(UnreadableReplyError):
        read_facts({"facts": []}, PAPER, 0.99)
    bad = reply()
    del bad["facts"]["switched"]
    with pytest.raises(UnreadableReplyError):
        read_facts(bad, PAPER, 0.99)
    bad = reply()
    bad["facts"]["switched"]["present"] = "no"
    with pytest.raises(UnreadableReplyError):
        read_facts(bad, PAPER, 0.99)
