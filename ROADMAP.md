# ROADMAP

Planned work, decided unless an item says otherwise, and not started. Where to
start on an item is in `HANDOVER.md`; remove an item here once it has landed.
The argument for every item is in the design,
`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md` (§ numbers below).

---

## Phase 1: bmlib additions (spec §7) — done

Every item merged in bmlib by 2026-10-31 (last: PR #586), all in bmlib 0.12.0
(published). What each gives phase 2: chunk storage and search (`bmlib::embed::store`; F16 ↔ Q8_0
makes every paper stale), `lookup_record`, the PICO agent, context windows
(the maximum or `None`, never a guess), streamed chat (only `openai`,
`deepseek` and `ollama` report usage), canonical text (code-point spans; a
PDF's text as extracted, so a table quote may fail to locate, §5.2),
`FullTextResult.jats_xml`, `find_passage` (fixture `passage_match.json`) and
the embedder with our GGUF (`hherb/snowflake-arctic-embed-l-v2.0-gguf`,
pinned by revision, size and sha256).

Open bmlib issues on landed items that phase 2 may meet: **#528**, **#529**
(`find_passage`), **#560** (`assess_full_text` strictness) and **#561**
(Rust's `chat()` retries nothing), all maintainer decisions, and **#651**
(the quote guard misses an arm swap and a spelled-out number; the threshold
waits on it) and **#647** (PICO drops a refused quote's reason); **#656** (the
2 M code-point HTML cap refuses pages that parse cheaply, such as one saved
with `data:` images); **#564** (Rust
`normalise_pmc_id` takes Unicode digits); **#566**, **#567** (context
window); **#572**, **#577**, and **#617**, **#618** on PR #615's code,
in our pin since bmlib #622, which also fixes #573 and #578 (PICO agent, `chat_json`); **#581**–**#583**, **#585** (record lookup), **#624** (its full-text sources untyped in Rust); **#605** (what a
page MuPDF cuts short should be; a notice today; #619, MuPDF's messages on a
failed read, closed by bmlib PR #670, in our pin); **#628** (headings missed once the body size is measured
right) and **#623** (what `pdf-4`'s review left); and minor
canonical-text issues **#548**, **#549** (in review, bmlib PR #675), **#552**, **#553**, **#557**,
**#558**. Opened by phase 2b: **#634** (a secret store for API keys); #635, #632,
#633, #563 and #654 closed by bmlib PRs #650, #648, #643, #653 and #655, all in our pin. Since: #636 (section names the PDF segmenter
missed) closed by bmlib PR #672, on bmlib main after our pin. Phase 2c meets **#665** (a defect in one batch of a partial
condensation is only a note; a decision, Python only), **#673** (a split
item's progress counts pieces against items; in review, bmlib PR #676, stacked
on #674) and ours **#60**; #638,
#640–#642, #646, #658/#576, #659, #662, #616, #664 and #667 closed by bmlib PRs
#639, #644, #657, #660, #661, #663, #666, #668 and #669, in our pin; #609
(no progress while a long paper is condensed) by bmlib PR #674, pinned at its
head until it merges.

## Phase 2: StudyAnalyzer on macOS (+ Linux, Windows)

Scope (§9): start screen, full-text retrieval with upload/paste fallback,
canonical text, Interrogator, PICO tab, settings, SQLite storage, the hand-off
file and link, style A light and dark, built sandbox-ready. Four plans, each in
`docs/superpowers/plans/`:

| Plan | Scope | Waits on |
|---|---|---|
| **2a** | Scaffold, storage, start screen, file/paste/identifier intake, retrieval, canonical text, full-text check, paper pane, contact email | **landed**, PR #10 |
| **2b** | Interrogator: chunks + embeddings and model download, whole-paper vs retrieval mode, streamed answers, quote marking, `find_passage` threshold by measurement, citations and highlights, provider/model settings, keychain | **landed**, PR #51 |
| **2c** | PICO tab | **landed**, PR #59 (the click-through is owed) |
| **2d** | `.studyanalyzer` hand-off file (container, versioning) and `studyanalyzer://` link, single instance | **landed**, PR #75 (the click-through is owed) |

## Phase 3: "Open in StudyAnalyzer" in MedicalFactChecker (macOS)

On the paper card and in the full-text viewer: write a hand-off file
(identifiers + PDF or stored full text) and open it with StudyAnalyzer; the
link alone without full text. First full round trip. MedicalFactChecker is not
sandboxed today; the hand-off must keep working once it is (§10).

## Phase 4: iPhone/iPad, then Android

Starts with a **measurement spike**: embedder speed and memory on a real phone
(F16 ≈ 1.1 GB, Q8_0 ≈ 0.6 GB). The plan then decides between Q8_0, embedding
only when retrieval mode is needed, or both. Then StudyAnalyzer on each
platform and MedicalFactChecker's sending side.

## Later, outside the design

Further analysis tabs (study assessment, PRISMA 2020, transparency, risk of
bias, retraction status, paper weight), each its own tab on the PICO pattern;
store submission (blocked on #20, MuPDF's licence); fine-tuning the embedder; other senders (bmlibrarian_lite's
desktop app).
