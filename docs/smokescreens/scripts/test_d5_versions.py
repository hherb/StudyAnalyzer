"""Checks of arm F's lists and second look. Run: <bmlib venv>/bin/python -m pytest test_d5_versions.py"""

import pytest
from bmlib.quality._full_text_agent import UnreadableReplyError
from bmlib.quality.outcome_agent import ProposedOutcome
from bmlib.quality.rob2_algorithm import Answer, Judgement

from d5_versions import PLAN_FACTS, assess_d5_by_versions, read_versions

PAPER = (
    "Pain was rated on a 0-10 visual analogue scale at 6 and 12 weeks. Function was measured with "
    "the Oswestry index at 12 weeks. Results: at 6 weeks mean pain was 3.1 versus 4.2 (p=0.01). "
    "Mortality was 2 of 40 and 3 of 40. Function improved in both groups."
)
OUTCOME = ProposedOutcome(name="pain at 6 weeks")


def reply(measurements=(), analyses=()) -> dict:
    return {
        "facts": {k: {"present": False, "quotes": []} for k in PLAN_FACTS},
        "other_measurements": list(measurements),
        "other_analyses": list(analyses),
    }


def item(label, named, result=()):
    return {"label": label, "named": list(named), "result": list(result)}


class FakeAgent:
    """Answers the first call with *first* and each second look from *looks*, in order."""

    def __init__(self, first, looks=()):
        self.replies = [first, *looks]
        self.calls = 0

    def _ask_until_readable(self, system, prompt, read, label):
        data = self.replies[self.calls]
        self.calls += 1
        return read(data)


def run(agent):
    return assess_d5_by_versions(agent, "T", PAPER, PAPER, OUTCOME, 0.99)


def test_an_item_whose_result_is_quoted_needs_no_second_look():
    agent = FakeAgent(reply([item("pain at 6 weeks", ["at 6 and 12 weeks"], ["mean pain was 3.1 versus 4.2"])]))
    d5, r = run(agent)
    assert agent.calls == 1
    assert r.lists["other_measurements"][0].reported
    assert d5.answers["5.2"].answer is Answer.NI


def test_an_item_the_second_look_finds_is_reported():
    # Mortality named, its result not quoted at first: the second look finds "2 of 40".
    agent = FakeAgent(
        reply([item("mortality", ["Mortality was 2 of 40"])]),
        looks=[{"found": True, "quotes": ["Mortality was 2 of 40 and 3 of 40."]}],
    )
    d5, r = run(agent)
    assert agent.calls == 2
    assert not r.lists["other_measurements"][0].not_reported
    assert d5.answers["5.2"].answer is Answer.NI


def test_an_item_both_looks_miss_makes_5_2_py():
    # Pain at 12 weeks is named, and the paper gives no result for it.
    agent = FakeAgent(
        reply([item("pain at 12 weeks", ["at 6 and 12 weeks"])]),
        looks=[{"found": False, "quotes": []}],
    )
    d5, r = run(agent)
    assert r.lists["other_measurements"][0].not_reported
    assert d5.answers["5.2"].answer is Answer.PY
    assert d5.judgement is Judgement.HIGH


def test_a_summary_without_numbers_is_not_a_result():
    agent = FakeAgent(
        reply(analyses=[item("Oswestry", ["the Oswestry index at 12 weeks"], ["Function improved in both groups."])]),
        looks=[{"found": True, "quotes": ["Function improved in both groups."]}],
    )
    d5, r = run(agent)
    assert r.lists["other_analyses"][0].not_reported
    assert d5.answers["5.3"].answer is Answer.PY


def test_an_item_named_by_no_found_passage_does_not_count():
    agent = FakeAgent(reply([item("pain at 24 weeks", ["at 24 weeks"])]))
    d5, r = run(agent)
    assert agent.calls == 1, "no second look for an item the paper does not name"
    assert not r.lists["other_measurements"][0].counts
    assert d5.answers["5.2"].answer is Answer.NI


def test_a_switch_with_a_reason_independent_of_the_results_is_no_y():
    switched = reply()
    switched["facts"]["switched"] = {"present": True, "quotes": ["Function was measured with the Oswestry index"]}
    d5, _ = run(FakeAgent(switched))
    assert d5.answers["5.2"].answer is Answer.Y
    switched["facts"]["change_explained"] = {"present": True, "quotes": ["at 12 weeks"]}
    d5, _ = run(FakeAgent(switched))
    assert d5.answers["5.2"].answer is Answer.NI


def test_an_unreadable_list_is_refused():
    bad = reply()
    bad["other_measurements"] = {"label": "x"}
    with pytest.raises(UnreadableReplyError):
        read_versions(bad, PAPER, 0.99)
    bad = reply([{"named": []}])
    with pytest.raises(UnreadableReplyError):
        read_versions(bad, PAPER, 0.99)
