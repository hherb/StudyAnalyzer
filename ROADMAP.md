# ROADMAP

Planned work, decided unless an item says otherwise, and not started. Where to
start on an item is in `HANDOVER.md`; remove an item here once it has landed.
The argument for every item is in the design,
`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md` (§ numbers below).

---

## Phase 1: bmlib additions (spec §7) — done

Every item merged in bmlib by 2026-10-31 (last: PR #586), all in bmlib 0.12.0
(published). What each gives phase 2: chunk storage and search
(`bmlib::embed::store`; F16 ↔ Q8_0 makes every paper stale), `lookup_record`,
the PICO agent, context windows
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
waits on it); #647 (PICO dropped a refused quote's reason) and #656 (the 2 M
code-point HTML cap) closed by bmlib PRs #727 and #740, in our pin;
**#566**, **#567** (context window); #572 (closed by bmlib PR #715, in our
pin); #577 (points 1–2) and #618 (both closed by bmlib PR #696, in
our pin; #577 point 3 stays open) and #617 (closed by bmlib PR #692, Python only), all on PR #615's code,
in our pin since bmlib #622, which also fixes #573 and #578 (PICO agent, `chat_json`); **#581**, **#582**, **#585** (record lookup; #583 closed by bmlib PR #679), **#605** (what a
page MuPDF cuts short should be; a notice today; #619, MuPDF's messages on a
failed read, closed by bmlib PR #670, in our pin); **#628** (headings missed once the body size is measured
right) and **#623** (what `pdf-4`'s review left); and minor
canonical-text issues **#552**, **#553**, **#557**,
**#558**. Opened by phase 2b: #634 (the API-key store) moved there in bmlib PR #687 (in our pin), which opened #686 (Windows names the item differently in each language; closed, in our pin); PR #92's review opened #688 (`eprintln!` warnings lost in a packaged app; closed by bmlib PR #693, in our pin, which filed **#694**, the FFI half) and #689 (damaged vs locked, closed by bmlib PR #690, in our pin); #635, #632,
#633, #563 and #654 closed by bmlib PRs #650, #648, #643, #653 and #655, all in our pin. Since: #636 (section names the PDF segmenter
missed), #548 and #549 (Markdown fences and quadratic emphasis) closed by
bmlib PRs #672 and #675, #624 (record full-text sources untyped in Rust)
and #564 (Rust `normalise_pmc_id` took Unicode digits) by #677 and #678, all
in our pin. Phase 2c met #665 (a defect in one batch of a partial
condensation is only a note; a decision, Python only; closed, in our pin); #638,
#640–#642, #646, #658/#576, #659, #662, #616, #664 and #667 closed by bmlib PRs
#639, #644, #657, #660, #661, #663, #666, #668 and #669, in our pin; #609
(no progress while a long paper is condensed) and #673 (a split item's
progress) by bmlib PRs #674 and #676, in our pin. #680 (no Rust model listing) by #682, in our pin; #681 (Rust's Gemini
chat URL had an extra `/v1`) closed by bmlib PR #732, in our pin. The 2b click-through's
round 2 led to bmlib PR #700 (a model's output limit beside its window, so a
reply's room can grow with the window; merged, in our pin). PR #100's review opened #702 (a non-streamed chat, every PICO
call, is capped at the 45 s request timeout, so a large `max_tokens` on a
slow thinking model times out instead of stopping at its length); fixed by
bmlib PR #706 (in our pin, with #709, which fixes #705: Anthropic refuses
`temperature`); its finalcheck's fixes are bmlib PR #712 (in our pin).
#705's fix opened #707 (`think=` refused by current Claude models; closed by
bmlib PR #711, in our pin), #708 (`chat_json`'s no-retry rule at
temperature 0 assumed greedy sampling; closed by bmlib PR #729, in our pin) and #707's opened #710 (current Claude models return no
thinking text unless asked for a summary; closed by bmlib PR #713, in our
pin). We send no `think=`. PR #105's review opened #744 (the PDF
sought beside JATS was never retried, and a failed search for it looked like
none; closed by bmlib PR #745, in our pin); PR #108's review opened
#746 (`http-408` read as final, and only the first failure beside JATS
judged; closed by bmlib PR #747, in our pin); PR #105's click-through opened
#748 (every PDF link of a CC BY paper answered a bot screen's 403; PMC's AWS
Open Data bucket serves it; closed by bmlib PR #749, in our pin; its follow-ups #750 and #751, the
bucket's record and JATS, closed by bmlib PR #752, in our pin). #649 and
#652 (a *Stop* left the server reading the prompt) closed by bmlib PR #714, in
our pin.

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

## Phase 3: the shared paper library, then "Open in StudyAnalyzer"

Design `docs/superpowers/specs/2026-11-13-shared-paper-library-design.md`
(approved 2026-11-13; it replaces the old phase 3, which copied each paper
into each app). A visible library folder the user picks once in each app;
plain files, write-once sources, one record per installation, nothing deleted
automatically. Four steps (design §8), each PR with its own plan: bmlib's
`library` module (1a, merged as PR #754; 1b `FullTextService` on it, merged as
PR #763; 1c bmlib-ffi, PR #764), StudyAnalyzer on it, MedicalFactChecker (macOS) on it,
then *Open in StudyAnalyzer* writing a hand-off file. iOS and Android join in
phase 4 (Files folder bookmark; Android's folder picker).

## Phase 4: iPhone/iPad, then Android

Starts with a **measurement spike**: embedder speed and memory on a real phone
(F16 ≈ 1.1 GB, Q8_0 ≈ 0.6 GB). The plan then decides between Q8_0, embedding
only when retrieval mode is needed, or both. Then StudyAnalyzer on each
platform and MedicalFactChecker's sending side.

## Later, outside the design

Further analysis tabs (study assessment, PRISMA 2020, transparency, risk of
bias, retraction status, paper weight), each its own tab on the PICO pattern
(bmlib PR #743, evidence weight phase 1, has RoB 2 and the ladder to build
on);
store submission (blocked on #20, MuPDF's licence); fine-tuning the embedder; other senders (bmlibrarian_lite's
desktop app).
