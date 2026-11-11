# Smoke tests

Small, fast experiments that check a piece of bmlib or StudyAnalyzer against real papers and a real
model before it is relied on. Each test has a findings file here, its scripts in
[scripts/](scripts/), and its samples, raw results and summaries under [data/](data/).

| Date | Test | Findings |
|---|---|---|
| 2026-11-10 | RoB 2 question wording: bmlib's own against the guidance's, on 2 × 10 RCTs and four models (plus Qwen3.8 on 2 RCTs), and a reworded 5.1 | [2026-11-10-rob2-wording.md](2026-11-10-rob2-wording.md) |

## The RoB 2 wording test

### What you need

- A Python with bmlib from branch `feat/evidence-weight` installed (for example the venv of a bmlib
  worktree on that branch, `uv pip install -e ".[all,dev]"`).
- `DEEPSEEK_API_KEY` in the environment for the default model, `deepseek:deepseek-flash`; other
  models are set by environment variables (below).
- A biasbuster checkout with the Eisele-Metzger benchmark: `dataset/eisele_metzger_benchmark.db`
  and `DATA/20240318_Data_for_analysis_full/fulltext/`. Default `~/src/biasbuster`; set
  `BIASBUSTER` otherwise.
- Your own copy of the RoB 2 full guidance PDF (Higgins et al., 22 August 2019), for wording B only.
  Its text is CC BY-NC-ND 4.0, so it is never committed: it lives in `private/`, which git ignores.

### Steps

```sh
cd docs/smokescreens
PY=~/src/bmlib-evidence-weight/.venv/bin/python   # any Python with bmlib feat/evidence-weight

# Wording B: the guidance's own text, one question at a time (private, not committed)
pdftotext -layout private/rob2.pdf private/rob2.txt
python3 scripts/extract_guidance.py private/rob2.txt private/guidance_layout.json

# Run: arms are A (bmlib wording, prompt v2), B (guidance), C (bmlib wording, prompt v3) and
# D (C with the reworded 5.1, now bmlib's v4; D5 only, written as "d5" rather than "run").
# A and C both use whatever wording the installed bmlib has; the letter only names the result files.
# E: D5 from quoted facts, answered by code (scripts/d5_facts.py), written as "d5" plus "facts".
# analyze.py reads A, B and C; D's files are counted directly (see the findings).
SMOKE_TAG=e $PY scripts/run.py E    # arm E on DeepSeek; other models as below, tag "<model>-e"
$PY scripts/compare_e.py            # arms C, E and E' side by side, per model and sample
$PY -m pytest scripts/test_d5_facts.py scripts/test_d5_versions.py
SMOKE_TAG=f3 $PY scripts/run.py F   # arm F ("not reported" checked); its version is F_VERSION
$PY scripts/review_f.py f3          # every arm F flag, to check by hand against the paper
$PY scripts/run.py --one            # one paper, arm A, as a check
$PY scripts/run.py C B              # first sample (10 RCTs), arms C and B, two runs each
SMOKE_SET=2 $PY scripts/run.py C B  # second sample, disjoint from the first

# Summaries
$PY scripts/analyze.py              # writes data/.../summary.json
SMOKE_SET=2 $PY scripts/analyze.py  # writes data/.../summary2.json

# Side-by-side page of both wordings (quotes the guidance: written to private/)
$PY scripts/export_bmlib_questions.py private/bmlib.json
python3 scripts/build_comparison.py private
```

Other models are set by environment variables; `SMOKE_TAG` names their own results folders and
summaries (`results-<tag>/`, `summary-<tag>.json`), for `run.py` and `analyze.py` alike:

```sh
# Ollama on another machine, through an SSH tunnel (ssh -N -L 11435:localhost:11434 dgx)
SMOKE_MODEL="ollama:gpt-oss:120B" SMOKE_TAG=gptoss120b OLLAMA_HOST=http://127.0.0.1:11435 \
  SMOKE_NUM_CTX=49152 SMOKE_WORKERS=2 $PY scripts/run.py C B
# An OpenAI-compatible server (oMLX), two papers, one run per arm
SMOKE_MODEL="openai:gemma-4-26b-a4b-it-4bit" SMOKE_TAG=gemma4-26b-a4b \
  SMOKE_BASE_URL=http://127.0.0.1:8000/v1 SMOKE_ONLY=RCT025,RCT045 SMOKE_REPS=1 SMOKE_WORKERS=1 \
  $PY scripts/run.py C B
SMOKE_TAG=gemma4-26b-a4b $PY scripts/analyze.py
```

`SMOKE_API_KEY` defaults to `local` when `SMOKE_BASE_URL` is set. `SMOKE_NUM_CTX` is Ollama's
context window: the longest paper plus prompt plus a 16,384-token reply.

`run.py` skips any result file that already exists and reruns any that holds an error, so a
stopped run resumes where it stopped.
`probe_d2.py` replays one D2 call with a high token limit; it is how the reasoning-token
truncation in the findings was found.

### Files

| File | Purpose |
|---|---|
| `scripts/run.py` | Runs bmlib's `Rob2Agent` per paper, swapping only the question text per arm |
| `scripts/analyze.py` | Agreement within and between arms and with Cochrane, per domain and per question |
| `scripts/probe_d2.py` | Replays one domain call to see output and reasoning tokens |
| `scripts/d5_facts.py`, `scripts/test_d5_facts.py` | Arm E: the facts prompt, the reply reader and the rules from facts to 5.1-5.3, with their checks |
| `scripts/compare_e.py` | Arm C against E, E' (the absence facts ignored) and F (versions 2 and 3) per model and sample |
| `scripts/d5_versions.py`, `scripts/test_d5_versions.py` | Arm F: other versions and the main analysis, each result checked at two looks |
| `scripts/review_f.py` | Lists arm F's flags with the paper's passages, for a hand check |
| `scripts/probe_json_mode.py` | Sends one domain call to an OpenAI-compatible server with and without JSON mode, saving both raw replies |
| `scripts/extract_guidance.py` | Splits the guidance PDF's three-column tables into question, paragraphs, options |
| `scripts/export_bmlib_questions.py` | Writes bmlib's question texts as JSON |
| `scripts/build_comparison.py`, `scripts/comparison_template.html` | Builds the side-by-side wording page |
| `data/2026-11-10-rob2-wording/` | `sample*.json`, `results*/` (one JSON per paper, arm and run), `summary*.json` |
