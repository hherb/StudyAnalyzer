"""Arms E′ and F version 3 re-derived with fact_guards' checks, from the stored runs: E′g and F3g
with the per-quote checks, E′p and F3p with the planning check as well (a measure, time point or
analysis must be stated as set in advance, not only described).

Run from docs/smokescreens with the bmlib venv's Python: <venv>/bin/python scripts/guard_report.py
Prints, per model and sample, D5 under E′, E′g, F3 and F3g; how often all four models agree on a
paper's D5; and every fact a check dropped, with its quotes, for a hand check of the checks.
"""

import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bmlib.quality.rob2_algorithm import Domain, domain_judgement  # noqa: E402
from compare_e import DATA, MODELS, d5_of, fmt, load, summary, without_absence_facts  # noqa: E402
from d5_facts import FACTS, answers_from  # noqa: E402
from fact_guards import guarded  # noqa: E402

ABSENCE = ("measurement_unreported", "analysis_unreported")
dropped: list[str] = []


def plan_present(record: dict, tag: str, need_plan: bool = False) -> dict[str, bool]:
    """The plan facts present after the checks; a dropped one is noted in ``dropped``."""
    present = {}
    for key in FACTS:
        if key in ABSENCE:
            continue
        fact = record["facts"][key]
        quotes = [c["text"] for c in fact["citations"]]
        kept = guarded(key, fact["present"], quotes, record["outcome"], need_plan)
        if fact["present"] and not kept:
            dropped.append(f"{tag} {record['rct']} run {record['rep']} {key}: {[q[:110] for q in quotes]}")
        present[key] = kept
    return present


def rederived(record: dict, present: dict[str, bool]) -> dict:
    """*record* with its D5 derived from *present*."""
    answers = answers_from(present)
    judgement = domain_judgement(Domain.D5, {q: a for q, (a, _) in answers.items()})
    d5 = {"status": "assessed", "answers": {q: {"answer": a.value} for q, (a, _) in answers.items()},
          "judgement": judgement.value}
    return {**record, "d5": d5}


def e_guarded(record: dict, tag: str, need_plan: bool = False) -> dict:
    """Arm E's run as E′ with the checks: absence facts ignored, plan facts checked."""
    if not record.get("facts"):
        return record
    present = plan_present(record, tag, need_plan) | {k: False for k in ABSENCE}
    return rederived(record, present)


def f_guarded(record: dict, tag: str, need_plan: bool = False) -> dict:
    """Arm F's run with the checks; "not reported" and the analysis check as stored."""
    if not record.get("facts"):
        return record
    present = plan_present(record, tag, need_plan)
    if record["facts"]["change_explained"]["present"]:
        present["switched"] = False
    present["measurement_unreported"] = any(
        i["not_reported"] for i in record["lists"]["other_measurements"])
    present["analysis_unreported"] = bool(record["analysis"] and record["analysis"]["counts"])
    return rederived(record, present)


def main() -> None:
    arms: dict[str, dict[str, dict]] = {}
    for model, (_c, e_suffix, _f2, f3_suffix) in MODELS.items():
        for sample, prefix in (("sample 1", "results"), ("sample 2", "results2")):
            e = load(DATA / f"{prefix}{e_suffix}", "E")
            f3 = load(DATA / f"{prefix}{f3_suffix}", "F")
            tag = f"{model}, {sample}"
            arms[tag] = {
                "E'": {k: without_absence_facts(r) for k, r in e.items()},
                "E'g": {k: e_guarded(r, f"E  {tag}") for k, r in e.items()},
                "F3": f3,
                "F3g": {k: f_guarded(r, f"F3 {tag}") for k, r in f3.items()},
                "E'p": {k: e_guarded(r, f"E'p {tag}", True) for k, r in e.items()},
                "F3p": {k: f_guarded(r, f"F3p {tag}", True) for k, r in f3.items()},
            }
            print(f"\n== {tag}")
            for arm, recs in arms[tag].items():
                s = summary(recs)
                print(f"  {arm:4} D5 [{fmt(s['judgements'])}]  same both runs {s['stable']}"
                      f"  vs Cochrane [{fmt(s['vs'])}]")
    print()
    for sample in ("sample 1", "sample 2"):
        for arm in ("E'", "E'g", "E'p", "F3", "F3g", "F3p"):
            per: dict[tuple, dict[str, str]] = {}
            for model in MODELS:
                for key, r in arms[f"{model}, {sample}"][arm].items():
                    d = d5_of(r)
                    if d and d.get("status") == "assessed":
                        per.setdefault(key, {})[model] = d["judgement"]
            full = [v for v in per.values() if len(v) == len(MODELS)]
            agree = sum(len(set(v.values())) == 1 for v in full)
            print(f"{sample} {arm:4} all four models agree on D5: {agree}/{len(full)}")
    quote_checks = [d for d in dropped if not d.startswith(("E'p", "F3p"))]
    print(f"\nFacts the per-quote checks dropped ({len(quote_checks)}), by fact: "
          f"{dict(Counter(d.split(' run ')[1].split(' ')[1].rstrip(':') for d in quote_checks))}")
    for line in quote_checks:
        print(" ", line)
    planning = [d for d in dropped if d.startswith("F3p")]
    print(f"\nFacts F3p's checks dropped ({len(planning)}); see them with --planning")
    if "--planning" in sys.argv:
        for line in planning:
            print(" ", line)


if __name__ == "__main__":
    main()
