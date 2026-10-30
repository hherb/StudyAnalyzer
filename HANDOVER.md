# HANDOVER

_Last updated: 2026-10-31._

Working notes for picking up in-flight work. Each section is one self-contained
slice: what's known, where to start, and how to verify. Remove a section once
its slice has landed, unless it holds calls the next phase builds on (as
*Phase 2a* does for 2b); add a new section when handing off new work. Planned
work not yet started is in `ROADMAP.md`.

---

## State

- On GitHub at `hherb/StudyAnalyzer` (public, AGPL-3.0). The design
  (`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`) is still
  marked *draft for approval*. Phase 2 is four plans (2a scaffold, storage
  and intake; 2b Interrogator; 2c PICO; 2d hand-off), the maintainer's call
  (2026-10-29).
- **Phase 1 is done**: every bmlib item (spec §7) is merged, the last on
  2026-10-30 (bmlib PR #586), all **unreleased** (bmlib 0.11.0 predates
  them). bmlib's own HANDOVER, ROADMAP and CHANGELOG hold the detail.
- **The bmlib pin (`rev` in the root `Cargo.toml`) is `3a4ea73`** (PR #586),
  past every phase 1 item. 2a needed no change for it; 2b meets its breaking
  chat changes (`ChatRequest.num_ctx`, `Protocol::Ollama`,
  `ProtocolError::Malformed`) when it first calls `chat()`.

## Phase 2a (merged)

**Phase 2a landed** as PR #10 (2026-10-29), after the plan's PR #6. Plan:
`docs/superpowers/plans/2026-10-29-phase2a-scaffold-and-intake.md`. 10 web
tests, 117 Rust tests, 1 ignored real-PDF test. Its reviews left #11–#14.
What it does: open a paper from a
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
- **A lookup by identifier has a total limit and a Cancel** (#7):
  `limit::Limit` (deadline `RETRIEVAL_DEADLINE`, 120 s, plus a shared cancel
  switch) and `limit::LimitedClient`, an `HttpClient` that refuses a request
  once the limit has stopped and clips each one's timeout to the time left.
  A request already running is not interrupted, so a cancel waits for it (at
  most bmlib's 45 s); a timeout ends at the deadline. A cancel is
  `OpenError::Cancelled` / kind `cancelled` and stores nothing; the UI shows
  no error for it. `AppState::begin_lookup`/`end_lookup` track the one
  running lookup for `cancel_lookup`. **For 2b**: chat and the model download
  can take a `Limit` the same way, but `LimitedClient` implements only `get`;
  add `post_json`/`post_json_stream` before routing chat through it.
- **`view::utf16_offsets` / `span_to_utf16`** are the only code-point → UTF-16
  conversion; 2b's highlights use them.

## Next

1. **bmlib fixes found by testing PR #16** (2026-10-31):
   - **bmlib #593** (closes bmlib #592), in review: Rust's cache never
     created `pdfs/`/`html/`, so a fresh install kept no retrieved PDF and
     every PDF-only paper read as "no full text". **Bump the pin once it
     merges.** Until then the dev machine has both directories made by hand
     in `~/Library/Caches/com.bmlibrarian.studyanalyzer/fulltext/`.
   - **bmlib #590**, a parallel session: Rust's PDF backend moves from
     PDFium to MuPDF (the `mupdf` crate, AGPL). PDFium puts a U+0002 box
     wherever a line ends in a hyphen (143 of 151 corpus PDFs) and duplicates
     characters. MuPDF also links statically, so **#8 (PDFium `dlopen`ed from
     Caches) goes away with it**: drop the PDFium setup in `src-tauri`
     (`PDFIUM_CACHE_ENV`, `PdfBackend`, `pdfium/`) when the pin takes #590.
     Soft hyphens are then joined (`pdf-2`), and stored `pdf-1` texts
     re-derive.
   - **bmlib #591**: running page headers and footers inside paragraphs
     (they break citation quotes that cross a page).
2. **Before plan 2b**: **#8**, unless bmlib #590 settles it. Also **#9** (two small UI leftovers from 2a's final review), and from the
   PR #10 review **#11** (failures still unshown or mis-worded), **#12** (pin
   the Rust↔TS JSON contract), **#13** (small hardening) and **#14** (missing
   doc comments).
3. **Identifier intake through `lookup_record`** (item 5, which landed after
   2a): pass a PPR accession as `ppr_id` (today 2a passes it as `pmid`, #13's
   last point), hand the record's `fulltext_sources`, `pmc_id`, `doi` and
   `pmid` to `fetch_fulltext`, show the title and `published_version`.
   `request_failed` at any source is worth a retry, never "no such paper".
   Fits 2b's plan or a small PR before it.
4. **Plan 2b** (Interrogator): write it in `docs/superpowers/plans/`, building
   on *Phase 2a*'s calls above and items 11, 6, 8 and 2 under *Landed*.
5. **Plan 2c** (PICO) after 2b; bmlib item 7 is in. 2d waits on **bmlib
   #563** (HTML nesting guard) and must settle how Rust trusts that a path
   came from the dialog or a drop.

## Owed by the maintainer

- **Approve the design**, or note what changes.
- **Decide bmlib #528 and #529** (`find_passage`: a superscript digit vs a
  plain one, and a quote whose seeds are all too common). Both touch how
  citations get accepted (§6.3).
- **Click through phase 2a**, if not done before the merge (plan Task 9
  Step 5; no agent could click the window): open a PDF, a JATS file, a saved
  HTML page, a pasted Markdown text; a PMID with free full text and one
  without; Wi-Fi off for the unreachable path; dark mode. (#7's *Cancel* was
  clicked through on 2026-10-31 and works.)
- **Decide bmlib #560**: how strict `assess_full_text` is (exact heading
  matches only, landing-page headings like `Cited by` excluded, `Methods
  summary` as Methods). It moves the shared fixture in both languages.
- **Decide bmlib #561**: should Rust's `chat()` retry a failed call as
  Python's SDKs do (and Python's Ollama provider too)?

## Landed (context)

Phase 1 items merged in bmlib, all **unreleased** (bmlib 0.11.0 predates them):

- **Item 8, a paper's chunks stored and searched**: PR #586.
  `bmlib::embed::store` (`ensure_schema`, `index_paper`, `search`,
  `delete_paper`); brute-force cosine in SQLite. For 2b: download with
  `download_model`, then `Embedder::load_model(&ARCTIC_EMBED_L_V2_F16, &path)`
  (never `load`; the `ModelFile` must be the one `path` was downloaded as);
  `ensure_schema` once on the app's database; `index_paper(db, paper_key,
  &canonical, &mut embedder, Some(&mut progress))` after converting the full
  text (show progress; `false` cancels and keeps the old set); `search(…, k)`
  in retrieval mode. `StoreError::Stale(_)` → re-index (re-convert first if
  `ConverterVersion` is among the fields); `NotIndexed` → index first;
  `Corrupt` → re-index. Switching F16 ↔ Q8_0 makes every paper stale. An
  indexed set is a dependent for `open::needs_rederive`. Open: bmlib #587,
  #588.
- **Item 5, one paper's record by identifier**: PRs #580 and #584.
  `bmlib::publications::lookup` / `lookup_record`; how 2a's intake should use
  it is *Next* 3. Open: bmlib #581, #582, #583, #585.
- **Item 7, the PICO agent**: PR #575. `PicoAgent` in `bmlib::quality`. For
  2c: run `check_suitability` on the abstract and show "not suitable" with a
  *run anyway* action calling `extract`; pass `CanonicalText.text` as `text`
  and `min_similarity` from 2b's measurement; label an `UNVERIFIED` part
  *Unsure – model could not reproduce <part> from text provided*.
  `REQUEST_FAILED` is retried, `UNUSABLE_REPLY` suggests another model,
  `TRUNCATED` a larger `max_tokens`. Configure `PicoAgent::pico_condenser`
  for long papers. Open: bmlib #572, #573, #574, #576, #577, #578.
- **Item 11, context windows**: PR #568 (design
  `~/src/bmlib/docs/superpowers/specs/2026-10-29-context-window-design.md`).
  What 2b builds on (the maintainer's calls 2026-10-29):
  - `LlmClient::context_window(model)` gives the model's **maximum**, or
    `None` when nobody publishes it. Never a guess. `NoSuchModel` means a
    mistyped model: tell the user. `MissingApiKey` asks for the key;
    `Transport` means the provider is unreachable; `UnknownProvider` and
    `Malformed` are reported as they are.
  - The **running size** is the user's setting, capped at the maximum. For
    Ollama it must be sent on every request as `ChatRequest.num_ctx`;
    without it Ollama runs at its own default (often 4096) and cuts the
    paper from the front without an error. Cloud providers ignore it.
  - Fit is tested against `min(setting, maximum)`, every window read as
    input plus answer. An unknown maximum means **retrieval mode until the
    user enters a number**, which is then both setting and cap.
  - Rust's Ollama now uses its native `/api/chat` (plain and streamed).
    Open: bmlib **#566** (Python's old metadata API guesses windows) and
    **#567** (Python and Rust route a bare provider name differently).
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
