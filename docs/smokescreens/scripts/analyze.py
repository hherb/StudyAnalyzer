"""Summarise the RoB 2 wording smoke test into summary.json (read by the report page)."""

import itertools
import os
import json
from collections import Counter, defaultdict
from pathlib import Path

DATA = Path(os.environ.get("SMOKE_DATA", Path(__file__).resolve().parent.parent / "data/2026-11-10-rob2-wording"))
SUFFIX = "" if os.environ.get("SMOKE_SET", "1") == "1" else os.environ["SMOKE_SET"]
TAG = f"-{os.environ['SMOKE_TAG']}" if os.environ.get("SMOKE_TAG") else ""
DOMAINS = ["d1", "d2", "d3", "d4", "d5", "overall"]
ORDER = {"low": 0, "some_concerns": 1, "high": 2}
ARMS = sorted({p.stem.split("_")[1][0] for p in (DATA / f"results{SUFFIX}{TAG}").glob("*.json")})
sample = [s["rct"] for s in json.loads((DATA / f"sample{SUFFIX}.json").read_text())]

runs = {}
for path in sorted((DATA / f"results{SUFFIX}{TAG}").glob("*.json")):
    r = json.loads(path.read_text())
    if r["rep"] >= 1 and r["rct"] in sample:
        runs[(r["rct"], r["arm"], r["rep"])] = r
sample = [rct for rct in sample if any(k[0] == rct for k in runs)]  # a SMOKE_ONLY run covers some papers only


def judgements(r):
    """{domain: judgement or None} for one run; None means failed or not assessed."""
    if "error" in r:
        return dict.fromkeys(DOMAINS)
    run = r["run"]
    out = {"d1": run["study"].get("judgement")}
    o = run["outcomes"][0]
    for d in o["domains"]:
        out[d["domain"].lower()] = d.get("judgement")
    out["overall"] = o.get("overall")
    return out


def answers(r):
    """{question: answer} over every assessed domain of one run."""
    if "error" in r:
        return {}
    run = r["run"]
    out = {}
    for d in [run["study"], *run["outcomes"][0]["domains"]]:
        for q, a in d.get("answers", {}).items():
            out[q] = a["answer"]
    return out


def kappa(pairs):
    """Linear-weighted Cohen's kappa over ordinal labels; None if undefined."""
    pairs = [(ORDER[a], ORDER[b]) for a, b in pairs if a and b]
    if not pairs:
        return None
    n, k = len(pairs), 3
    w = lambda i, j: 1 - abs(i - j) / (k - 1)
    po = sum(w(a, b) for a, b in pairs) / n
    ca, cb = Counter(a for a, _ in pairs), Counter(b for _, b in pairs)
    pe = sum(w(i, j) * ca[i] * cb[j] for i in range(k) for j in range(k)) / n / n
    return None if pe == 1 else round((po - pe) / (1 - pe), 2)


def agreement(pairs):
    pairs = [(a, b) for a, b in pairs if a and b]
    return {"n": len(pairs), "agree": sum(a == b for a, b in pairs),
            "pct": round(100 * sum(a == b for a, b in pairs) / len(pairs)) if pairs else None,
            "kappa": kappa(pairs)}


J = {k: judgements(r) for k, r in runs.items()}
A = {k: answers(r) for k, r in runs.items()}
coch = {k[0]: r["cochrane"] for k, r in runs.items()}


def judgement_pairs(left, right, domains=DOMAINS):
    out = []
    for rct in sample:
        for (a1, r1), (a2, r2) in [(left, right)]:
            if (rct, a1, r1) in J and (rct, a2, r2) in J:
                out += [(J[(rct, a1, r1)][d], J[(rct, a2, r2)][d]) for d in domains]
    return out


comparisons = {
    "A run 1 vs A run 2 (noise)": judgement_pairs(("A", 1), ("A", 2)),
    "B run 1 vs B run 2 (noise)": judgement_pairs(("B", 1), ("B", 2)),
    "A vs B (same run number)": judgement_pairs(("A", 1), ("B", 1)) + judgement_pairs(("A", 2), ("B", 2)),
    "C run 1 vs C run 2 (noise)": judgement_pairs(("C", 1), ("C", 2)),
    "C vs B (same run number)": judgement_pairs(("C", 1), ("B", 1)) + judgement_pairs(("C", 2), ("B", 2)),
    "C vs A (same run number)": judgement_pairs(("C", 1), ("A", 1)) + judgement_pairs(("C", 2), ("A", 2)),
}
vs_cochrane = {}
for arm in ARMS:
    pairs, by_domain, direction = [], defaultdict(list), Counter()
    for rct in sample:
        for rep in (1, 2):
            if (rct, arm, rep) not in J:
                continue
            for d in DOMAINS:
                m, c = J[(rct, arm, rep)][d], coch[rct][d]
                pairs.append((m, c))
                by_domain[d].append((m, c))
                if m and c:
                    direction["same" if m == c else "stricter" if ORDER[m] > ORDER[c] else "more lenient"] += 1
    vs_cochrane[arm] = {"all": agreement(pairs), "by_domain": {d: agreement(p) for d, p in by_domain.items()},
                        "direction": dict(direction)}

per_arm = {}
for arm in ARMS:
    keys = [k for k in runs if k[1] == arm]
    labels, failures, errors, seconds = Counter(), [], [], []
    ans, quotes = Counter(), Counter()
    for k in keys:
        r = runs[k]
        seconds.append(r["seconds"])
        if "error" in r:
            errors.append(f"{k[0]} run {k[2]}: {r['error']}")
            continue
        for d, j in J[k].items():
            labels[j or "not assessed"] += 1
        run = r["run"]
        for d in [run["study"], *run["outcomes"][0]["domains"]]:
            if d["status"] != "assessed":
                failures.append(f"{k[0]} run {k[2]} {d['domain']}: {d.get('reason', '')[:160]}")
            for a in d.get("answers", {}).values():
                ans[a["answer"]] += 1
                quotes["verified"] += len(a["citations"])
                quotes["not found"] += len(a["quotes_not_found"])
                quotes["contradicted"] += len(a["contradicted"])
                quotes["dropped"] += a["quotes_dropped"]
        if run["outcomes"][0].get("escalation_error"):
            failures.append(f"{k[0]} run {k[2]} escalation: {run['outcomes'][0]['escalation_error'][:160]}")
    per_arm[arm] = {"runs": len(keys), "labels": dict(labels), "answers": dict(ans), "quotes": dict(quotes),
                    "failures": failures, "errors": errors,
                    "median_seconds": sorted(seconds)[len(seconds) // 2] if seconds else None}

# Question by question: how often two arms give the same answer (same run number, both answered).
def question_agreement(x, y):
    out = {}
    for rct, rep in itertools.product(sample, (1, 2)):
        a, b = A.get((rct, x, rep), {}), A.get((rct, y, rep), {})
        for q in sorted(set(a) | set(b), key=lambda s: tuple(map(int, s.split(".")))):
            e = out.setdefault(q, {"both": 0, "same": 0, "only_" + x: 0, "only_" + y: 0, "pairs": Counter()})
            if q in a and q in b:
                e["both"] += 1
                e["same"] += a[q] == b[q]
                if a[q] != b[q]:
                    e["pairs"][f"{a[q]}→{b[q]}"] += 1
            elif q in a:
                e["only_" + x] += 1
            else:
                e["only_" + y] += 1
    for e in out.values():
        e["pairs"] = dict(e["pairs"].most_common())
    return out


questions = {f"{x} vs {y}": question_agreement(x, y) for x, y in (("A", "B"), ("C", "B")) if x in ARMS and y in ARMS}

# Per RCT and domain: every run's judgement next to Cochrane's.
table = []
for rct in sample:
    row = {"rct": rct, "outcome": next(r["outcome"] for k, r in runs.items() if k[0] == rct), "cells": {}}
    for d in DOMAINS:
        row["cells"][d] = {"cochrane": coch[rct][d],
                           **{f"{arm}{rep}": J.get((rct, arm, rep), {}).get(d) for arm in ARMS for rep in (1, 2)}}
    table.append(row)

summary = {"sample": sample, "runs": len(runs),
           "comparisons": {k: agreement(v) for k, v in comparisons.items()},
           "vs_cochrane": vs_cochrane, "per_arm": per_arm, "questions": questions, "table": table}
(DATA / f"summary{SUFFIX}{TAG}.json").write_text(json.dumps(summary, ensure_ascii=False, indent=1))
print(json.dumps({k: summary[k] for k in ("runs", "comparisons")}, indent=1))
for arm in ARMS:
    p = per_arm[arm]
    print(arm, "vs cochrane", vs_cochrane[arm]["all"], vs_cochrane[arm]["direction"])
    print("  labels", p["labels"], "answers", p["answers"])
    print("  quotes", p["quotes"], "median s", p["median_seconds"], "failures", len(p["failures"]), "errors", len(p["errors"]))
