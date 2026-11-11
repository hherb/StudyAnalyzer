"""Checks of fact_guards, from the cases the arm F review found. Run with pytest."""

from fact_guards import guarded, names_outcome, states_a_plan, supports


def test_a_plan_is_dated_by_a_date_or_a_before_phrase_not_by_a_registration_number():
    assert not supports("plan_dated_early", "This study is registered with ClinicalTrials.gov, NCT04546581.", "x")
    assert not supports("plan_dated_early",
                        "The primary analysis was planned after approximately 24 primary end-point events", "x")
    assert supports("plan_dated_early", "registered (identifier 113021; November 4, 2012)", "x")
    assert supports("plan_dated_early", "The trial was prospectively registered (ISRCTN123).", "x")
    assert supports("plan_dated_early", "registered on 03/04/2015", "x")
    assert supports("plan_dated_early", "The analysis plan was finalised before unblinding.", "x")
    assert supports("plan_dated_early", "approved (8883-12, 20121212)", "x")


def test_a_switch_needs_the_language_of_change():
    assert not supports("switched", "There was one death in the control group and none in the case group", "x")
    assert not supports("switched", "one death in the control group; this difference was insignificant (P = 0.495).", "x")
    assert supports("switched", "The reported outcome differs from the registered one.", "x")
    assert supports("switched", "The primary outcome was changed from pain to function after review.", "x")
    assert supports("switched", "Originally registered as a secondary outcome", "x")


def test_a_measure_must_name_the_outcome_assessed():
    primary = "The primary endpoint was an ordinal outcome based on the patient's clinical status on day 7"
    assert not names_outcome(primary, "deaths up to day 28")
    assert names_outcome("Secondary outcomes included death from any cause by day 28.", "deaths up to day 28")
    assert names_outcome("all-cause mortality at the end of follow-up was recorded", "all-cause mortality at the end of follow-up")
    # A name with no distinctive word cannot be checked, and passes.
    assert names_outcome("anything", "score at 6 months")


def test_a_time_point_needs_a_time():
    assert supports("timepoint_fixed", "These scales were completed at baseline and at the end of the program.", "x")
    assert not supports("timepoint_fixed", "The primary outcome was quality of life.", "x")


def test_unchecked_facts_pass_and_one_passing_quote_is_enough():
    assert supports("plan_cited", "anything", "x")
    assert guarded("switched", True, ["Deaths were 2 and 3.", "The outcome was changed."], "x")
    assert not guarded("switched", True, ["Deaths were 2 and 3."], "x")
    assert not guarded("switched", False, ["The outcome was changed."], "x")


def test_the_planning_check_wants_a_plan_not_a_description():
    assert not states_a_plan(["Statistical analysis was performed by the IBM SPSS Statistics."])
    assert not states_a_plan(["Assessments were performed at baseline, 7- and 12-weeks follow-up."])
    assert states_a_plan(["The secondary efficacy endpoints as prespecified in the statistical analysis plan were:"])
    assert states_a_plan(["A Poisson regression was used as the primary efficacy analysis model."])
    assert states_a_plan(["The primary analysis compared proportions of patients."])
    # The plan may be stated in one quote of the fact and the outcome named in another.
    quotes = ["The secondary endpoints as prespecified were:", "Mean change in mean diurnal IOP at day 8"]
    assert guarded("measure_fixed", True, quotes, "change of mean diurnal intraocular pressure (IOP)", need_plan=True)
    assert not guarded("measure_fixed", True, quotes[1:], "change of mean diurnal intraocular pressure (IOP)", need_plan=True)
    assert guarded("measure_fixed", True, quotes[1:], "change of mean diurnal intraocular pressure (IOP)")
