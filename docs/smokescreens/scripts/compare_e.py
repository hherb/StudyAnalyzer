"""Arm E (D5 from quoted facts) against arm C (D5 asked directly), per model and sample.

E' re-applies E's rules to the stored facts with the two "unreported" facts ignored: a quote can
show where a measurement is named, never that its result is missing, so those facts are the
model's unchecked word, and a PY on 5.2 or 5.3 makes D5 high.

Run from docs/smokescreens with the bmlib venv's Python: <venv>/bin/python scripts/compare_e.py
Prints, per model and sample: the 5.1-5.3 answers and D5 judgements of each arm, how often the
two runs of a paper give the same D5 judgement, agreement with Cochrane's D5, failed domains,
and for E how many claimed facts the paper bore out.
"""

import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bmlib.quality.rob2_algorithm import Domain, domain_judgement  # noqa: E402
from d5_facts import FACTS, answers_from  # noqa: E402

#: The facts E' ignores: claims of absence, which no quote can bear out.
ABSENCE_FACTS = ("measurement_unreported", "analysis_unreported")

DATA = Path(__file__).resolve().parent.parent / "data/2026-11-10-rob2-wording"
#: model -> (arm C folder suffix, arm E folder suffix); "" is DeepSeek, the default model.
MODELS = {
    "DeepSeek": ("", "-e"),
    "gpt-oss 120B": ("-gptoss120b", "-gptoss120b-e"),
    "gpt-oss 20B": ("-gptoss20b", "-gptoss20b-e"),
    "Gemma 4 26B": ("-gemma4-26b-a4b", "-gemma4-26b-a4b-e"),
}
#: Judgements in order of concern, for "stricter" and "more lenient".
ORDER = {"low": 0, "some_concerns": 1, "high": 2}


def d5_of(record: dict) -> dict | None:
    """The D5 domain of a result file, from arm C's whole run or arm D/E's D5 alone."""
    if "d5" in record:
        return record["d5"]
    if "run" not in record:
        return None
    return next(d for d in record["run"]["outcomes"][0]["domains"] if d["domain"] == "D5")


def without_absence_facts(record: dict) -> dict:
    """*record* with its D5 re-derived from its facts, the absence facts ignored (E')."""
    if not record.get("facts"):
        return record
    present = {k: record["facts"][k]["present"] and k not in ABSENCE_FACTS for k in FACTS}
    answers = {q: {"answer": a.value} for q, (a, _) in answers_from(present).items()}
    judgement = domain_judgement(Domain.D5, {q: a for q, (a, _) in answers_from(present).items()})
    return {**record, "d5": {"status": "assessed", "answers": answers, "judgement": judgement.value}}


def load(folder: Path, arm: str) -> dict[tuple[str, int], dict]:
    """(rct, rep) -> record for *arm*'s files in *folder*."""
    out = {}
    for path in sorted(folder.glob(f"*_{arm}?.json")):
        r = json.loads(path.read_text())
        out[(r["rct"], r["rep"])] = r
    return out


def fmt(c: Counter) -> str:
    return ", ".join(f"{k} {v}" for k, v in sorted(c.items(), key=lambda kv: -kv[1]))


def summary(records: dict[tuple[str, int], dict]) -> dict:
    """The counts compared for one arm, model and sample."""
    answers = {q: Counter() for q in ("5.1", "5.2", "5.3")}
    judgements, failed, vs = Counter(), 0, Counter()
    by_rct: dict[str, dict[int, str]] = {}
    claimed = borne = 0
    for (rct, rep), r in records.items():
        d = d5_of(r)
        if d is None or d.get("status") != "assessed":
            failed += 1
            continue
        for q in answers:
            answers[q][d["answers"][q]["answer"]] += 1
        j = d["judgement"]
        judgements[j] += 1
        by_rct.setdefault(rct, {})[rep] = j
        cochrane = r["cochrane"]["d5"].lower().replace(" ", "_")
        diff = ORDER[j] - ORDER.get(cochrane, ORDER[j])
        vs["same" if diff == 0 else "stricter" if diff > 0 else "more lenient"] += 1
        for f in (r.get("facts") or {}).values():
            claimed += f["claimed"]
            borne += f["present"]
    pairs = [js for js in by_rct.values() if len(js) == 2]
    same = sum(1 for js in pairs if js[1] == js[2])
    return {
        "answers": answers, "judgements": judgements, "failed": failed, "vs": vs,
        "stable": f"{same}/{len(pairs)}", "claimed": claimed, "borne": borne,
    }


def main() -> None:
    for model, (c_suffix, e_suffix) in MODELS.items():
        for sample, prefix in (("sample 1", "results"), ("sample 2", "results2")):
            c = load(DATA / f"{prefix}{c_suffix}", "C")
            e = load(DATA / f"{prefix}{e_suffix}", "E")
            if not e:
                continue
            print(f"\n== {model}, {sample}")
            e_prime = {k: without_absence_facts(r) for k, r in e.items()}
            for arm, recs in (("C", c), ("E", e), ("E'", e_prime)):
                s = summary(recs)
                print(f"  {arm}: 5.1 [{fmt(s['answers']['5.1'])}]  5.2 [{fmt(s['answers']['5.2'])}]"
                      f"  5.3 [{fmt(s['answers']['5.3'])}]")
                print(f"     D5 [{fmt(s['judgements'])}]  same both runs {s['stable']}"
                      f"  vs Cochrane [{fmt(s['vs'])}]  failed {s['failed']}"
                      + (f"  facts borne out {s['borne']}/{s['claimed']}" if arm == "E" else ""))


if __name__ == "__main__":
    main()
