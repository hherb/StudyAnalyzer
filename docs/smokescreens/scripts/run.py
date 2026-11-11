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
from bmlib.quality.rob2_questions import Domain
from bmlib.quality.rob2_questions import question_text as bmlib_text
from d5_facts import E_VERSION, assess_d5_by_facts
from d5_versions import F_VERSION, assess_d5_by_versions

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

MODEL = os.environ.get("SMOKE_MODEL", "deepseek:deepseek-flash")
#: Results of a model other than the default go to their own folders (results-<tag>, results2-<tag>).
TAG = os.environ.get("SMOKE_TAG", "")
OUT = DATA / (f"results{SUFFIX}" + (f"-{TAG}" if TAG else ""))
OLLAMA_HOST = os.environ.get("OLLAMA_HOST")  # e.g. http://localhost:11435 through an SSH tunnel
#: An OpenAI-compatible server for "openai:<model>" (e.g. oMLX at http://127.0.0.1:8000/v1).
BASE_URL = os.environ.get("SMOKE_BASE_URL")
API_KEY = os.environ.get("SMOKE_API_KEY", "local" if BASE_URL else None)
#: Comma-separated RCT ids: run only these papers of the sample.
ONLY = [x for x in os.environ.get("SMOKE_ONLY", "").split(",") if x]
#: Ollama's context window: the longest paper (~61k characters, ~16k tokens) plus prompt and a
#: 16384-token reply with the model's reasoning. Ignored by hosted providers.
NUM_CTX = int(os.environ["SMOKE_NUM_CTX"]) if os.environ.get("SMOKE_NUM_CTX") else None
SEED = 20261110
N_RCTS = 10
REPS = int(os.environ.get("SMOKE_REPS", "2"))
WORKERS = int(os.environ.get("SMOKE_WORKERS", "4"))  # one Ollama model serves one request at a time
MIN_SIMILARITY = 0.99  # StudyAnalyzer's MIN_QUOTE_SIMILARITY
MAX_TOKENS = 8192  # above the agent's 4096 default, so a long D2 reply is not cut off in either arm
NO_CONDENSING = 1_000_000  # characters; every paper is sent whole, so condensing adds no noise

sys.path.insert(0, str(BIASBUSTER / "studies/eisele_metzger_replication"))
from exclusions import WRONG_PAPER_RCTS  # noqa: E402

guide = json.loads(GUIDANCE.read_text())


def guidance_text(q: str) -> str:
    g = guide[q]
    return "\n".join([g["title"], *g["paras"], f"Response options: {g['options']}"])


#: Candidate wording for 5.1 (arm D): the 5.2/5.3 evidence ladder applied to it. Adopted as bmlib's
#: prompt version 4 (7b55d695), so with that bmlib installed, arm C asks it too.
Q_5_1_D = """\
Did this result come from an analysis that was planned, and written down, \
before anyone could see the outcome data split by group?
Look in: the trial registration, the protocol or statistical analysis plan \
the paper cites, and the dates it gives for them. Compare their outcome \
measure and analysis with the ones reported. A paper that only calls an \
outcome or analysis "prespecified", or cites a plan without saying what it \
fixes for this outcome, gives you nothing to compare.
Y: the paper cites such a plan, with a date that places it before the data \
by group were seen, and the plan fixes this outcome's measure and analysis; \
the report follows it, and any change was made before the data were seen or \
for a stated reason that has nothing to do with the results.
PY: the paper describes what the plan fixes for this outcome (its measure \
and analysis) closely enough to compare, and the report follows it, but only \
the paper's word places the plan before the data by group were seen: it \
cites no registration, protocol or analysis plan for it, or cites one \
without a date that does.
PN: the report seems to depart from the plan, or there are signs that the \
plan was changed after the data by group could be seen.
N: the report departs from the plan without a reason unrelated to the \
results, or the plan was written or changed after the outcome data were \
seen.
NI: the paper names no plan settled in advance; or it names one but gives \
too little of what the plan fixes for this outcome to compare, even when it \
calls the outcome or analysis "prespecified"; or the plan's only date places \
it after the data by group were seen, such as a registration made late."""

ARMS = {
    "A": bmlib_text,
    "B": guidance_text,
    "C": bmlib_text,  # bmlib wording after the smoke-test fixes (prompt v3)
    "D": lambda q: Q_5_1_D if q == "5.1" else bmlib_text(q),  # C with the candidate 5.1; D5 only
    "E": bmlib_text,  # D5 from quoted facts and code (d5_facts.py); asks no question text
    "F": bmlib_text,  # E with "not reported" checked by two looks (d5_versions.py)
}
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
    if path.exists() and "error" not in json.loads(path.read_text()):
        return  # done; a run that errored (a dropped tunnel, say) is tried again
    local.arm = arm
    started = time.monotonic()
    record = {"rct": item["rct"], "arm": arm, "rep": rep, "model": MODEL, "outcome": item["outcome"],
              "prompt_version": ra.ROB2_PROMPT_VERSION,
              "cochrane": item["cochrane"]}
    try:
        title, text = paper(item["rct"])
        record["chars"] = len(text)
        tokens = {} if arm in ("C", "E", "F") or SET != "1" else {"max_tokens": MAX_TOKENS}  # C and E take the agent's default
        if NUM_CTX:
            tokens["num_ctx"] = NUM_CTX
        agent = ra.Rob2Agent(
            LLMClient(ollama_host=OLLAMA_HOST, base_url=BASE_URL, api_key=API_KEY), MODEL, **tokens,
            condense_config=ProcessingConfig(max_context_chars=NO_CONDENSING),
        )
        outcome = ProposedOutcome(name=item["outcome"])
        if arm == "D":  # 5.1 touches only D5: ask that domain alone, the way assess() does
            d5 = agent._domain(Domain.D5, title.strip(), text.strip(), text, outcome, MIN_SIMILARITY, {})
            record["d5"] = d5.to_dict()
        elif arm == "E":  # D5 from facts the paper bears out, answered by code
            d5, facts = assess_d5_by_facts(agent, title.strip(), text.strip(), text, outcome, MIN_SIMILARITY)
            record["e_version"] = E_VERSION
            record["d5"] = d5.to_dict()
            record["facts"] = {k: f.to_dict() for k, f in facts.items()} if facts else None
        elif arm == "F":  # E with "not reported" borne out by the paper, at two looks
            d5, reply = assess_d5_by_versions(agent, title.strip(), text.strip(), text, outcome, MIN_SIMILARITY)
            record["f_version"] = F_VERSION
            record["d5"] = d5.to_dict()
            record["facts"] = {k: f.to_dict() for k, f in reply.facts.items()} if reply else None
            record["lists"] = (
                {k: [i.to_dict() for i in items] for k, items in reply.lists.items()} if reply else None
            )
        else:
            run = agent.assess(title, text, [outcome], min_similarity=MIN_SIMILARITY)
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
    if not ONLY:
        (DATA / f"sample{SUFFIX}.json").write_text(json.dumps(sample, ensure_ascii=False, indent=1))
    if ONLY:
        sample = [s for s in sample if s["rct"] in ONLY]
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
