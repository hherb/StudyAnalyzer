# HANDOVER

_Last updated: 2026-10-29._

Working notes for picking up in-flight work. Each section is one self-contained
slice: what's known, where to start, and how to verify. Remove a section once
its slice has landed; add a new section when handing off new work. Planned
work not yet started is in `ROADMAP.md`.

---

## State

- On GitHub at `hherb/StudyAnalyzer` (public, AGPL-3.0). The design
  (`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`) is still
  marked *draft for approval*. Phase 2 is four plans (2a scaffold, storage
  and intake; 2b Interrogator; 2c PICO; 2d hand-off), the maintainer's call
  (2026-10-29).
- **Phase 1 runs in bmlib** (`~/src/bmlib`), under bmlib's own HANDOVER,
  ROADMAP, CHANGELOG and `docs/SESSION-RULES.md`. This file only tracks where
  it stands.

## In flight

**Phase 2a, in review**: branch `feat/phase2a`, stacked on the plan's PR #6
(`docs/phase2-plan`). Plan:
`docs/superpowers/plans/2026-10-29-phase2a-scaffold-and-intake.md`. All six
gates pass (10 web tests, 117 Rust tests, 1 ignored real-PDF test). Built task by task, each reviewed,
then a whole-branch review and two fix waves (the second from the PR #10
review; what it left is #11–#14). What it does: open a paper from a
file (dialog or drop), a paste or a PMID/DOI/PMC ID; retrieve full text
(JATS first, else a cached PDF; never the abstract); decode any encoding
strictly (BOM → UTF-8 → declared → chardetng guess, shown to the user);
convert to bmlib's canonical text; run `assess_full_text` (a failed check
opens with a banner and a stored "this is the whole paper"); store everything
in SQLite (originals as BLOBs, a connection per command, WAL, IMMEDIATE
transactions); show the paper in style A, light and dark.

Calls made while building, for 2b to know:
- **Identifiers** are stored canonical: PMID digits without leading zeros or
  `PPR`+digits; DOI lower-cased; PMC `PMC`+ASCII digits without leading zeros.
  A JATS source's own identifiers are recorded too, so a paper opened by PMID
  and later by its DOI is one paper. An identifier another paper holds is
  skipped with a warning, except on `attach`: a JATS file whose own
  identifiers contradict the paper is refused (`OpenError::WrongPaper`).
- **Retrieval** parses bmlib's exhaustion wording to tell "no free full text"
  (not retried) from "could not reach the sources" (retried) — a `shortcut:`
  until **bmlib #565** types it. With a DOI or PMID, bmlib answers `Ok` with a
  bare link even when every tier faulted; only its "No full text found …
  N attempts failed" log line tells, so `FullTextSource::fetch` reads it
  (`retrieval::faulted_exhaustion`) and forwards all of bmlib's log lines to
  the app log.
- **Re-deriving** passes the stored file name as the title hint again and
  carries the user's "this is the whole paper" over to the new text.
- **Re-derive**: a newer converter re-derives from the source the stored text
  names, only while nothing points into it (`open::needs_rederive`'s
  `has_dependents`, always `false` in 2a: **2b must pass the real answer** once
  chunks or citations exist).
- **`retry::with_backoff`** blocks its thread (use `spawn_blocking`); 2b wraps
  Rust `chat()` in it while **bmlib #561** stands.
- **`view::utf16_offsets` / `span_to_utf16`** are the only code-point → UTF-16
  conversion; 2b's highlights use them.

**Owed before merge:** the by-hand checks of plan Task 9 Step 5 (no agent
could click the window): open a PDF, a JATS file, a saved HTML page, a pasted
Markdown text; a PMID with free full text and one without; Wi-Fi off for the
unreachable path; dark mode.

## Next

1. **bmlib item 11, a model context-window table** (the maintainer's call
   2026-10-29): spec §6.2 decides whole-paper vs retrieval mode from it. It
   blocks 2b. Rust and Python, shared fixture: read bmlib's `HANDOVER.md` and
   `docs/SESSION-RULES.md`, plan in `~/src/bmlib/docs/superpowers/plans/`,
   branch off bmlib's `main`, gates in both languages (`CLAUDE.md` § Gates).
2. **Plan 2b** (Interrogator) once 2a merges. Before it: **#7** (retrieval
   needs a total deadline or cancel) and **#8** (PDFium is `dlopen`ed from
   Caches; bundle it before any signed, sandboxed build). Also **#9** (two small UI
   leftovers from 2a's final review), and from the PR #10 review **#11**
   (failures still unshown or mis-worded), **#12** (pin the Rust↔TS JSON
   contract), **#13** (small hardening) and **#14** (missing doc comments).
3. Items 5, 7 (blocks 2c) and 8 in bmlib can go in parallel. 2d waits on
   **bmlib #563** (HTML nesting guard) and must settle how Rust trusts that a
   path came from the dialog or a drop.

## Owed by the maintainer

- **Approve the design**, or note what changes.
- **Decide bmlib #528 and #529** (`find_passage`: a superscript digit vs a
  plain one, and a quote whose seeds are all too common). Both touch how
  citations get accepted (§6.3).
- **Click through phase 2a** (see *In flight*) and merge PR #6, then 2a's PR.
- **Decide bmlib #560**: how strict `assess_full_text` is (exact heading
  matches only, landing-page headings like `Cited by` excluded, `Methods
  summary` as Methods). It moves the shared fixture in both languages.
- **Decide bmlib #561**: should Rust's `chat()` retry a failed call as
  Python's SDKs do (and Python's Ollama provider too)?

## Landed (context)

Phase 1 items merged in bmlib, all **unreleased** (bmlib 0.11.0 predates them):

- **Item 6, streamed chat**: PR #562. `LlmClient::chat_stream(&request,
  &mut on_delta)` calls `on_delta` per `StreamDelta` (`Text` or `Thinking`)
  and returns `chat()`'s `LLMResponse`. For phase 2:
  - **Stop button**: return `ControlFlow::Break(())` from `on_delta`; the
    response holds what arrived, `stop_reason = "cancelled"`.
  - **A stream that fails after it opens, or breaks off, is `ChatError::Stream`**,
    never a short answer; an HTTP or connection failure before it opens is
    `Transport`.
    The deltas already shown are an incomplete answer: say so in the chat, and
    do not check citations against it as if it were whole.
  - **No tools on a stream** (refused with
    `ChatError::StreamingToolsUnsupported`). The text is exactly what was streamed,
    so the quote-marking format (§6.2, builder's call) must survive streaming:
    check citations on the final `content`, not on deltas.
  - Opening is retried (3 tries, backoff); the body gets
    `UreqClient::stream_timeout_seconds` (600). Rust's `chat()` itself retries
    nothing (**bmlib #561**), so wrap PICO's `chat()` calls in a retry unless
    #561 is fixed first.
  - Among OpenAI-compatible providers, only `openai`, `deepseek` and `ollama`
    report usage on a stream; the others report 0 tokens.
- **Item 4c, canonical text from HTML, and a full-text check**: PR #559.
  `canonical_from_html(html)` (`"html-1"`) reads a saved or pasted page by
  fixed rules (the fullest `<article>`, else `<main>`, else `<body>`),
  html5ever in both languages (Rust feature `html`, Python extra `html`).
  For phase 2: **run `assess_full_text` on every canonical text** before the
  Interrogator opens (it is how "never fall back to the abstract" holds for an
  uploaded or pasted page; what `False` does is phase 2's call); decode the
  bytes yourself and cap a page's size, though only a nesting guard bounds
  the parse (**#563**); prefer `canonical_from_markdown` for a
  MedicalFactChecker hand-off, which carries Markdown too. Open: **#557**, **#558** (after 2026-11-04),
  **#560** (owed above).
- **Item 4b, canonical text from PDF**: PR #556.
  `canonical_from_segmented(document)` takes the segmenter's
  `SegmentedDocument` (`"pdf-1"`). For phase 2: pass the paper's known title
  to `segment_document()` as `metadata["title"]`; front matter and the
  no-headings fallback are untitled sections; the text is as extracted, so a
  line-break hyphen reads `dose- response`, a title read off page 1 is often
  printed again in the front matter, and a table quote may fail to locate
  (§5.2). PyMuPDF (Python) and PDFium (Rust) can segment one PDF
  differently; the app uses Rust's.
- **Item 3, RAG chunker**: PR #554. `chunk_canonical(canonical,
  max_chars=1000, overlap_chars=100)` takes a `CanonicalText` (not its
  string) and returns chunks that are each a code-point span into the
  canonical text plus the index of their section. For phase 2: a chunk never
  crosses a section and never passes `max_chars` (an over-long sentence is cut
  at whitespace, or mid-token at the cap where a piece has none); the title
  and headings are not chunked, so prefix the heading when embedding if
  wanted; store `CHUNKER_VERSION` with each chunk beside the canonical text's
  `converter_version`.
- **Item 4a, canonical text (model, JATS, Markdown, plain text)**: PR #547.
  `bmlib.fulltext.canonical` lays a paper out as one string, the title and
  every heading and block a code-point span. Item 4 is three PRs (the
  maintainer's call, 2026-10-28): 4a, then 4b PDF and 4c HTML. The JATS text
  keeps everything `to_html()` shows but the metadata lines, references
  included. For phase 2: store `CanonicalText.to_dict()` with its
  `converter_version`; a table row is a `table_row` block with tab-separated
  cells; Markdown keeps a section's `#` level after the title (so its top
  sections may be level 2, where JATS starts at 1). Open: bmlib #548, #549,
  #552, #553.
- **Item 9, retrieval returns its source**: PR #545. `FullTextResult.jats_xml`
  holds the JATS bytes as served; the cache keeps them in `jats/`; a PDF stays
  at `file_path`. An old HTML-only cache entry is served with `jats_xml=None`,
  not re-fetched. So **StudyAnalyzer gives `FullTextService` its own cache
  directory** in app storage, and treats `jats_xml=None` on a `fulltext`
  result as "no structured source".

- **Item 2, passage matcher**: PR #526, `find_passage` in Python and Rust,
  fixture `tests/fixtures/passage_match.json`, plus the bmlibrarian oracle. Open
  issues: #528, #529. PR #544 fixed #533 (the timing tests now bound windows
  scored, not wall-clock time).
- **Item 1, embedder**: PR #531 (`bmlib.embed`; Rust over `llama-cpp-2`,
  Python over llama-cpp-python as `bmlib[embed]`), then #534 (refuses NaN,
  infinite or zero vectors) and #535 (error types matched across the two
  languages). Shared contract `embedding_contract.json`.
- **Item 10, our own GGUF**: in PR #531.
  `scripts/convert_embedding_model.py`; F16 and Q8_0 published at
  `hherb/snowflake-arctic-embed-l-v2.0-gguf`, pinned by revision, size and
  sha256.

Phase 2 depends on bmlib through a path or git dependency until a release
includes these (spec §11).
