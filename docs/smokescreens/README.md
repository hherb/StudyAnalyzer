# Smoke tests

Small, fast experiments that check a piece of bmlib or StudyAnalyzer against real papers and a real
model before it is relied on. Each test has a findings file here, its scripts in
[scripts/](scripts/), and its samples, raw results and summaries under [data/](data/).

| Date | Test | Findings |
|---|---|---|
| 2026-11-10 | RoB 2 question wording: bmlib's own against the guidance's, on 2 × 10 RCTs | [2026-11-10-rob2-wording.md](2026-11-10-rob2-wording.md) |

## The RoB 2 wording test

### What you need

- A Python with bmlib from branch `feat/evidence-weight` installed (for example the venv of a bmlib
  worktree on that branch, `uv pip install -e ".[all,dev]"`).
- `DEEPSEEK_API_KEY` in the environment (the test uses `deepseek:deepseek-flash`; change `MODEL` in
  `scripts/run.py` for another).
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

# Run: arms are A (bmlib wording, prompt v2), B (guidance), C (bmlib wording, prompt v3).
# A and C both use whatever wording the installed bmlib has; the letter only names the result files.
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

`run.py` skips any result file that already exists, so a stopped run resumes where it stopped.
`probe_d2.py` replays one D2 call with a high token limit; it is how the reasoning-token
truncation in the findings was found.

### Files

| File | Purpose |
|---|---|
| `scripts/run.py` | Runs bmlib's `Rob2Agent` per paper, swapping only the question text per arm |
| `scripts/analyze.py` | Agreement within and between arms and with Cochrane, per domain and per question |
| `scripts/probe_d2.py` | Replays one domain call to see output and reasoning tokens |
| `scripts/extract_guidance.py` | Splits the guidance PDF's three-column tables into question, paragraphs, options |
| `scripts/export_bmlib_questions.py` | Writes bmlib's question texts as JSON |
| `scripts/build_comparison.py`, `scripts/comparison_template.html` | Builds the side-by-side wording page |
| `data/2026-11-10-rob2-wording/` | `sample*.json`, `results*/` (one JSON per paper, arm and run), `summary*.json` |
