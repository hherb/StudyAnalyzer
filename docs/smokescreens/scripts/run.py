"""RoB 2 wording smoke test: bmlib's wording (A) against the guidance's own (B).

Same agent, model, sampling, papers and outcome for both; only the text of the 22 signalling
questions differs. Each arm runs twice so the run-to-run noise floor is visible.
Experiment, not product code. Run with a Python that has bmlib from branch feat/evidence-weight
installed, and DEEPSEEK_API_KEY set. See ../README.md.
"""

import json
import os
import random
import sqlite3
import sys
import threading
import time
import traceback
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import bmlib.quality.rob2_agent as ra
from bmlib.context_processor.data_types import ProcessingConfig
from bmlib.fulltext.canonical.jats import canonical_from_jats
from bmlib.fulltext.jats_parser import JATSParser
from bmlib.llm.client import LLMClient
from bmlib.quality.outcome_agent import ProposedOutcome
from bmlib.quality.rob2_questions import question_text as bmlib_text

HERE = Path(__file__).resolve().parent
SMOKESCREENS = HERE.parent
#: Samples, raw results and summaries of this smoke test (committed).
DATA = Path(os.environ.get("SMOKE_DATA", SMOKESCREENS / "data/2026-11-10-rob2-wording"))
#: The RoB 2 guidance, extracted from your own copy of the PDF (CC BY-NC-ND 4.0: never committed).
GUIDANCE = Path(os.environ.get("ROB2_GUIDANCE_JSON", SMOKESCREENS / "private/guidance_layout.json"))
#: biasbuster checkout with the Eisele-Metzger benchmark database and full texts.
BIASBUSTER = Path(os.environ.get("BIASBUSTER", Path.home() / "src/biasbuster"))
DB = BIASBUSTER / "dataset/eisele_metzger_benchmark.db"
FULLTEXT = BIASBUSTER / "DATA/20240318_Data_for_analysis_full/fulltext"
SET = os.environ.get("SMOKE_SET", "1")  # "2": a second sample, disjoint from the first
SUFFIX = "" if SET == "1" else SET
OUT = DATA / f"results{SUFFIX}"

MODEL = "deepseek:deepseek-flash"
SEED = 20261110
N_RCTS = 10
REPS = 2
WORKERS = 4
MIN_SIMILARITY = 0.99  # StudyAnalyzer's MIN_QUOTE_SIMILARITY
MAX_TOKENS = 8192  # above the agent's 4096 default, so a long D2 reply is not cut off in either arm
NO_CONDENSING = 1_000_000  # characters; every paper is sent whole, so condensing adds no noise

sys.path.insert(0, str(BIASBUSTER / "studies/eisele_metzger_replication"))
from exclusions import WRONG_PAPER_RCTS  # noqa: E402

guide = json.loads(GUIDANCE.read_text())


def guidance_text(q: str) -> str:
    g = guide[q]
    return "\n".join([g["title"], *g["paras"], f"Response options: {g['options']}"])


ARMS = {"A": bmlib_text, "B": guidance_text, "C": bmlib_text}  # C: bmlib wording after the smoke-test fixes (prompt v3)
RUN_ARMS = sys.argv[1:] if sys.argv[1:] and sys.argv[1] != "--one" else ["A", "B"]
local = threading.local()
ra.question_text = lambda q: ARMS[local.arm](q)  # the agent's prompt builder reads this name


def pick_rcts() -> list[dict]:
    con = sqlite3.connect(DB)
    rows = con.execute(
        "select rct_id, title, outcome_text from benchmark_rct order by rct_id"
    ).fetchall()
    cochrane = {}
    for rct, domain, judgment in con.execute(
        "select rct_id, domain, judgment from benchmark_judgment where source='cochrane' and valid=1"
    ):
        cochrane.setdefault(rct, {})[domain] = judgment
    eligible = [
        {"rct": r, "title": t, "outcome": o, "cochrane": cochrane[r]}
        for r, t, o in rows
        if r not in WRONG_PAPER_RCTS
        and (FULLTEXT / r / "paper.jats.xml").exists()
        and o
        and all(cochrane.get(r, {}).get(d) for d in ("d1", "d2", "d3", "d4", "d5", "overall"))
    ]
    if SET != "1":
        first = {x["rct"] for x in json.loads((DATA / "sample.json").read_text())}
        eligible = [e for e in eligible if e["rct"] not in first]
    print(f"eligible RCTs: {len(eligible)}", flush=True)
    return random.Random(SEED + int(SET) - 1).sample(eligible, N_RCTS)


def paper(rct: str) -> tuple[str, str]:
    canonical = canonical_from_jats(JATSParser((FULLTEXT / rct / "paper.jats.xml").read_bytes()).parse())
    span = canonical.title_span
    title = canonical.text[span.start : span.end] if span else ""
    return title, canonical.text


def one(item: dict, arm: str, rep: int) -> None:
    path = OUT / f"{item['rct']}_{arm}{rep}.json"
    if path.exists():
        return
    local.arm = arm
    started = time.monotonic()
    record = {"rct": item["rct"], "arm": arm, "rep": rep, "model": MODEL, "outcome": item["outcome"],
              "prompt_version": ra.ROB2_PROMPT_VERSION,
              "cochrane": item["cochrane"]}
    try:
        title, text = paper(item["rct"])
        record["chars"] = len(text)
        tokens = {} if arm == "C" or SET != "1" else {"max_tokens": MAX_TOKENS}  # C takes the agent's new default
        agent = ra.Rob2Agent(
            LLMClient(), MODEL, **tokens,
            condense_config=ProcessingConfig(max_context_chars=NO_CONDENSING),
        )
        run = agent.assess(title, text, [ProposedOutcome(name=item["outcome"])], min_similarity=MIN_SIMILARITY)
        record["run"] = run.to_dict()
    except Exception as exc:  # recorded, never a score
        record["error"] = f"{type(exc).__name__}: {exc}"
        record["traceback"] = traceback.format_exc()
    record["seconds"] = round(time.monotonic() - started, 1)
    path.write_text(json.dumps(record, ensure_ascii=False, indent=1))
    print(f"{item['rct']} {arm}{rep} {record['seconds']}s {'ERROR ' + record['error'] if 'error' in record else 'ok'}",
          flush=True)


def main() -> None:
    OUT.mkdir(exist_ok=True)
    sample = pick_rcts()
    (DATA / f"sample{SUFFIX}.json").write_text(json.dumps(sample, ensure_ascii=False, indent=1))
    print("sample:", [s["rct"] for s in sample], flush=True)
    jobs = [(s, arm, rep) for s in sample for rep in range(1, REPS + 1) for arm in RUN_ARMS]
    with ThreadPoolExecutor(WORKERS) as pool:
        for future in [pool.submit(one, *job) for job in jobs]:
            future.result()
    print("done", flush=True)


if __name__ == "__main__":
    if sys.argv[1:2] == ["--one"]:  # quick check on one paper, arm A only
        OUT.mkdir(exist_ok=True)
        s = pick_rcts()[0]
        one(s, "A", 0)
    else:
        main()
