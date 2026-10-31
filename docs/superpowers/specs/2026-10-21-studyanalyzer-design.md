# StudyAnalyzer — design

_2026-10-21. Status: draft for approval._

## 1. What it is, and why

StudyAnalyzer is a companion app for reading **one** biomedical paper closely.
Another app — first MedicalFactChecker — hands it a paper; StudyAnalyzer gets
the full text and lets the user question it.

The moment it exists for: the user has found a paper in MedicalFactChecker and
the abstract does not tell them what they need. The first thing they ask for
is a **PICO breakdown** (often not obvious from the abstract); the next is a
targeted question such as *"is there anything specifically about patients over
75?"*. They act on the answer only if they can see **where in the paper it
comes from** — the same citation principle as MedicalFactChecker, but here the
cited passage is highlighted in the paper itself.

It is written in **Rust with a Tauri 2 GUI**, and it uses **bmlib** (Rust)
wherever it can. It borrows heavily from **bmlibrarian**, the parent project
of bmlibrarian_lite. Anything StudyAnalyzer needs that another app could use
is built **in bmlib first**, not in this repository.

**The apps** the family maintains move to **Rust + Tauri**, the default
multi-platform tooling now; the Python + PyQt/PySide apps (bmlibrarian,
bmlibrarian_lite's desktop) are retired eventually. **bmlib itself stays a
two-language library**: Python bmlib has users of its own, so every bmlib
addition lands in **Rust and Python, kept on par wherever possible**, held
together by shared contract fixtures as the retraction work (bmlib#504) was.

**Distribution:** eventually the **Apple App Store and Google Play**, and
always buildable from source on GitHub. Design for the App Store's sandbox
from the start (§4.1).

## 2. Goals, non-goals, anti-goals

**Goals (version 1)**

- Receive a paper from another app on each target platform: identifiers, plus
  the full text when the sender has it.
- Get the full text: what the sender passed, else bmlib's retrieval chain, else
  the user uploads a file or pastes text.
- The **Interrogator** tab: chat about the paper, every claim backed by a quote
  that is found in the paper and highlighted there.
- A **PICO** tab: the first ready-made analysis, in its own tab — the
  pattern every later analysis follows.
- Remember everything about a paper so reopening it is instant and offline.
- A polished UI from the start (see §6.5).

**Non-goals (version 1)**

- Further analyses (study assessment, PRISMA 2020, transparency, risk of bias,
  retraction status, paper weight). Each comes later **as its own tab**.
- Sending anything back to the calling app.
- Rendering the original PDF pages. The paper pane shows clean, reflowed text.
- Figures. Tables are included as text (§5.2) so they can be quoted.
- A shared vector database with bmlibrarian or other apps. The embedding
  choices below keep it possible; building it is not part of this work.
- Comparing several papers at once.
- Fine-tuning the embedding model on biomedical literature (planned later; §7
  item 10 prepares for it).

**Anti-goals** — failure even if everything technically works:

- Answering from the abstract, or from the model's general knowledge, while
  looking as if it answered from the full text.
- Showing a citation whose text is not in the paper.
- Silently dropping a claim's support: if a quote cannot be located, the user
  is told.

## 3. Platforms and order

macOS first; Linux and Windows come with it at little extra cost; then
iPhone/iPad; then Android. Tauri's mobile support is younger than its desktop
support, and the embedder has never been measured on a phone (§9).

## 4. Getting a paper in

### 4.1 From another app

The sender passes:

- whatever identifiers it has — **PMID** (which may hold a preprint accession
  such as `PPR…`), **DOI**, **PMC ID**;
- **the full text, when it has it**.

Both apps will be sandboxed (App Store, Play Store), so **a file path cannot
be passed**: a sandboxed receiver may not read another app's container, and on
mobile that storage is also clearable. The design therefore uses **one
hand-off file**, delivered by each platform's own "open this file with that
app" route, which grants the receiver access to that one file:

| Platform | Route |
|---|---|
| macOS | the sender opens the hand-off file with StudyAnalyzer (`NSWorkspace.open(_:withApplicationAt:…)`); Tauri receives it as an opened file |
| Linux / Windows | the same, as a file argument / file association |
| iPhone / iPad | "Open in" / share sheet, StudyAnalyzer registering the document type |
| Android | `ACTION_VIEW` / `ACTION_SEND` with a `content://` URI and a read grant (FileProvider) |

**The hand-off file** (registered type, e.g. extension `.studyanalyzer`)
carries the identifiers, minimal metadata the sender already has (title), the
sender's name, and the full text **inside the file** — the PDF bytes, or the
JATS / markdown / HTML text — with its format and where the sender got it.
*Builder's call:* the container (e.g. a zip with a JSON manifest, or JSON with
an embedded payload) and its versioning. My call: one format on every
platform, so MedicalFactChecker's sending code is the same idea everywhere.

**Identifiers only** (the sender has no full text) may also come as a link:
`studyanalyzer://open?pmid=…&doi=…&pmcid=…` (Tauri deep-link plugin), with
single-instance handling so a second hand-off goes to the running app.

StudyAnalyzer **copies the received full text into its own storage
immediately**; the granted access may end moments later.

Bundle id `com.bmlibrarian.studyanalyzer` and scheme `studyanalyzer` are my
call.

**Trust boundary.** Any program can open that URL or send that file. Validate
every identifier against its format; accept only a well-formed hand-off file
of a supported type and below a size cap; never execute or follow anything in
it (no paths or URLs inside the hand-off file are dereferenced).

### 4.2 Opened on its own

StudyAnalyzer is also usable without another app: a start screen offers

- **recent papers** (everything previously opened),
- **open a file** (PDF, JATS XML, HTML, Markdown, plain text; drag-and-drop on desktop),
- **paste text**,
- **enter an identifier** (PMID / DOI / PMC ID) — my call; it is the same path as a hand-off without a file.

### 4.3 Finding the full text

1. Full text passed by the sender → use it.
2. Otherwise → bmlib's retrieval chain (`FullTextService::fetch_fulltext`:
   Europe PMC XML, NCBI PMC, PMC-ID discovery, Europe PMC PDF, Unpaywall, DOI).
3. Otherwise → a screen saying no full text was found, offering **upload a
   file** or **paste the text**.

Metadata (title, journal, year, authors) comes from a by-identifier lookup —
which bmlib lacks today (§7).

**Never fall back to the abstract.** Without full text the Interrogator does
not open on that paper.

## 5. The paper's text

### 5.1 One canonical text per paper

Every format is converted into one **canonical text**: a title plus an ordered
list of sections (heading, paragraphs). It is what the paper pane shows and
what every offset in the system — chunks, citations, highlights — points into.

- **JATS XML** → bmlib's JATS reader (`JATSArticle`), rendered to the
  canonical text (bmlib has only an HTML renderer today — new, §7).
- **PDF** → bmlib's MuPDF extraction and section segmenter (`pdf` feature).
- **HTML** → main-content extraction (new; bmlib has none).
- **Markdown** → headings become sections, inline markup stripped.
- **Plain text** → one section, paragraphs by blank lines.

The canonical text is stored, together with the **converter version that
produced it**. Chunks, citations and highlights record which canonical text
they belong to, so a later converter change cannot leave highlights pointing
at the wrong place. *Builder's call:* whether a new converter version
re-derives (and re-locates highlights) or keeps the old text for papers that
already have conversations.

Offsets are in **one unit, chosen once** (bmlib's chunker counts Unicode code
points; JavaScript counts UTF-16 units). The conversion happens at the
Rust/frontend boundary, nowhere else.

### 5.2 Tables

JATS tables are included in the canonical text as plain rows, so a quote from
a table can be located and highlighted. Subgroup results often live in tables.
Nice table rendering is later work.

**PDF tables (my call):** version 1 takes PDF text as bmlib's extraction gives
it. That extraction is prose-oriented (`pdf_text.rs::render_html` "recovers
the prose, not the layout"), so table cells may come out run together. Real
PDF table detection is a separate, measured bmlib project for later. Until
then a table quote from a PDF may fail to locate and show the "could not be
found" note — honest, not silent.

## 6. The Interrogator

### 6.1 Layout (decided)

Chat on the left, the paper on the right (clean reflowed text, section
headings). Tabs across the top: **Interrogator**, **PICO**, then later
analyses.

- Every claim in an answer carries a numbered citation.
- Clicking a citation scrolls the paper to the passage.
- The passage for the **current** citation is **dark yellow**; passages cited
  **earlier in the conversation stay pale yellow**, so the paper accumulates
  everything the conversation relied on.
- **Phone:** the two halves become a *Chat | Paper* switch; tapping a citation
  flips to *Paper*, scrolled to the passage.

Mockups from the brainstorming session are the reference for look and
behaviour: `mockups/interrogator-layout-v2.html` (layout) and
`mockups/visual-style.html` (style A). They are HTML fragments; their own
`<style>` blocks carry the colours. The *PICO* tab they show is right (§6.4).

### 6.2 How an answer is made (decided)

- **Whole paper when it fits.** If the canonical text plus the conversation and
  room for the answer fit in the chosen model's context window, the model reads
  the whole paper on every question.
- **Retrieved passages when it does not.** Otherwise the paper's chunks are
  searched by embedding and the best passages are sent.
- **Fit is decided automatically** from the model's context window (provider
  metadata where available, else a known table), with a user setting that
  **lowers** the limit — for local hardware that cannot run the model's full
  window. My call.
- Answers are grounded in the paper only. If the paper does not say, the
  answer says so.
- Follow-up questions see the earlier turns.
- Answers **stream** in as the model writes them; citations attach once the
  quotes have been checked. (bmlib has no streaming yet — §7.)

*Builder's call:* the retrieval details (number of passages, query rewriting
as bmlibrarian_lite does, whether to add keyword search), and how the model
marks its quotes so they survive streaming (inline markers vs a trailing
structured block).

### 6.3 Citations (decided)

The model must back each claim with a quote copied from the paper. Each quote
is located in the canonical text by a port of bmlibrarian's **`find_passage`**
(`src/bmlibrarian/utils/passage_match.py`):

- It forgives what LLM copies typically get wrong — case, whitespace,
  typographic quotes and dashes, accents — and accepts a quote whose similarity
  to a span of the paper reaches a threshold, so a few wrong characters still
  pass. **Near-verbatim is accepted.**
- It returns the **paper's own span and wording**; that span is what gets
  highlighted and stored, never the model's copy.
- A quote below the threshold (a paraphrase) loses its citation, and the answer
  shows a visible note — e.g. *"1 quote could not be found in the paper"*.

The acceptance threshold is a constant, set from measurement during the port.

### 6.4 The PICO tab (decided: a tab, not a button)

PICO is the first **analysis tab**, and sets the pattern later analyses
(study assessment, PRISMA, transparency…) follow. *My calls on the shape:*

- Same split as the Interrogator: the analysis on the left as a structured
  card — **population, intervention, comparison, outcome**, plus study type and
  sample size — and the paper on the right.
- **Each part carries its citation(s)**; clicking one highlights the passage
  (dark yellow current, pale yellow for the analysis's other citations).
- The analysis runs the first time the tab is opened for a paper, is stored,
  and shows instantly afterwards; a *re-run* action repeats it, e.g. with
  another model. Every run is kept (model and date recorded).

It ports bmlibrarian's `agents/pico_agent.py`:

- the suitability check (not every paper has a PICO: reviews, case reports…),
  answered plainly when it does not apply;
- the per-field guidance, *"not clearly stated"* for a missing part, and the
  per-part confidence rubric;
- the full text is never truncated (whole-paper mode, or map-reduce over
  chunks when it does not fit).

Unlike bmlibrarian's, every PICO part carries verified quotes (§6.3).

### 6.5 Look (decided: style A, "Journal")

The paper reads like a printed article: serif body text on warm off-white,
navy accents, sans-serif chat that stays quiet so the paper leads. Highlights:
current `#e8b931`-ish, past `#f6e7b0`-ish. A dark variant follows the system
setting. Frontend stack: **Svelte 5 + TypeScript + Vite**, as in the
maintainer's other Tauri apps (`~/src/hanzitutor`, `~/src/bottie`).

### 6.6 Settings

- **Provider and model** — anything bmlib's `LlmClient` supports (Anthropic,
  OpenAI-compatible, Ollama…), plus model parameters where they apply.
- The **most recent choice is remembered**.
- The context-limit override (§6.2).
- API keys in the **OS keychain**, never in the database or a plain file;
  environment variables (`ANTHROPIC_API_KEY`, …) honoured as a fallback. My call.

The maintainer's experience: models under ~20B parameters do poorly on this
task. Users without strong hardware will use a cloud provider. No model is
hard-wired.

## 7. Work in bmlib first

Each item is its own bmlib PR, with tests, before or alongside the app work
that needs it. The maintainer's rule: shared logic lives in bmlib so other
apps get it too.

| # | Piece | Source / notes |
|---|---|---|
| 1 | **Embedder**: snowflake-arctic-embed-l-v2.0, in-process via `llama-cpp-2`, F16 GGUF pinned by repository + revision + sha256, CLS pooling, first **256 dims, L2-renormalised**; queries use the model's query prompt, documents none | From `~/src/bmlibrarian/spikes/embedding-throughput/` (`llamacpp/`, `FINDINGS.md`). Feature-gated. Check: cosine vs the spike's reference vectors |
| 2 | **Passage matcher**: Rust port of `find_passage` (normalisation with offset map, seed voting, window refinement) | `bmlibrarian/src/bmlibrarian/utils/passage_match.py`; its 47 tests are the oracle. Consider also `complete_sentence_span` / `occurs_once` from `agents/citation_agent.py` |
| 3 | **RAG chunker**: sentence-aware, section-labelled chunks with offsets into the canonical text | Size close to bmlibrarian's `embeddings/chunk_embedder.py` (~1 000 chars, 100 overlap) so a future shared vector store stays compatible. Existing `TextChunker` (10k chars) is for map-reduce, not this |
| 4 | **Canonical text** from JATS (new text renderer over `JATSArticle`), PDF (segmenter output), HTML (new), Markdown, plain text — with section spans | §5 |
| 5 | **Metadata by identifier** (PMID / DOI / PMC ID) | `parse_article_set` exists; a single lookup call does not |
| 6 | **Streaming chat** in `LlmClient` for Anthropic, OpenAI-compatible and Ollama | bmlib is blocking (`ureq`); a callback-per-delta API fits it without going async |
| 7 | **PICO agent** port | `bmlibrarian/src/bmlibrarian/agents/pico_agent.py` (27 tests), extended with quotes |
| 8 | **Vector storage** for chunks + embeddings in SQLite | For one paper, brute-force cosine is enough; sqlite-vec is the builder's call |
| 9 | **Retrieval returns its source**: `FullTextService` also hands back the raw JATS XML or PDF bytes it fetched, not only `FullTextResult.html` | Today `fetch_europepmc` renders JATS to HTML and drops the parsed article, PDFs come back as flat `<p>` prose, and the cache keeps HTML only — none of which yields §5's sections. My call: extend `FullTextService` (option a) rather than fetching JATS in the app or parsing bmlib's HTML back |
| 10 | **Own GGUF of the embedding model**: convert snowflake-arctic-embed-l-v2.0 ourselves (llama.cpp's converter), F16 (and Q8_0 for phase 4), check it against the reference vectors, publish under the maintainer's Hugging Face account, pin that | The spike's GGUF is a third party's (`Casual-Autopsy/…`); pinning protects integrity, not availability. Licence: **Apache-2.0** (base XLM-RoBERTa: MIT) — conversion, hosting and later fine-tuning are allowed; keep the licence and attribution. The conversion is a reproducible, committed script, since fine-tuning will rerun it |

**Every item lands in both Rust and Python bmlib, on par wherever possible,**
in the same PR, with shared contract fixtures both implementations must pass
(the retraction-checking precedent). Where the two cannot match exactly — e.g.
the embedder's runtime (Python would use llama.cpp's Python binding with the
same GGUF) — the contract states the tolerance (cosine against reference
vectors) instead of byte equality. Item 10 is a language-neutral script.

Neither the Python nor the Rust bmlib has `find_passage` or an embedder
today; nothing in this list is "wire up what is there". `find_passage` moves
from bmlibrarian into Python bmlib, and the Rust port is checked against it.

bmlib is **AGPL-3.0**; StudyAnalyzer links it and is AGPL-3.0 too. App Store
and Play distribution of both is workable because the maintainer holds their
copyright; permissive third-party pieces (llama.cpp MIT, the model Apache-2.0,
the libraries MuPDF bundles such as FreeType and HarfBuzz) need their notices
shipped. MuPDF (Rust bmlib's PDF engine since bmlib PR #596, Python bmlib's
through PyMuPDF) is AGPL-3.0 too, but Artifex holds its copyright, so that
argument does not cover it: store distribution needs Artifex's commercial
licence or another answer (issue #20).

## 8. Storage

Everything learned about a paper is stored in StudyAnalyzer's own SQLite
database (via bmlib's `db` layer) in the app's data directory — the
maintainer's rule is that nothing found about a document lives only in memory:

- the paper: identifiers, metadata, where the full text came from;
- the original full-text file (copied in) and the canonical text with its
  converter version;
- chunks and their embeddings;
- conversations: questions, answers, located citations (span + quote), the
  model used;
- the embedding model file (downloaded on first use, checksum-verified, with
  progress shown).

A paper is identified by its PMID / DOI / PMC ID when it has one (so a second
hand-off of the same paper reopens it), else by a generated id.

*My call:* one ongoing conversation per paper is shown; "new conversation"
starts another and keeps the old one.

## 9. Phases

Each phase gets its own implementation plan.

1. **bmlib additions** (§7), Rust and Python together. Items 1–4, 6, 9 and 10
   block phase 2; 5, 7, 8 can land during it. Python parity roughly doubles
   this phase.
2. **StudyAnalyzer on macOS** (+ Linux, Windows): start screen, full-text
   retrieval and upload/paste, canonical text, Interrogator with whole-paper
   and retrieval modes, verified citations and highlights, PICO tab,
   settings, storage, the hand-off file and link (§4.1), style A light and
   dark. Built sandbox-ready (App Store entitlements, data in the app's
   container) even before it is submitted.
3. **"Open in StudyAnalyzer" in MedicalFactChecker on macOS** — on the paper
   card and in the full-text viewer — writing a hand-off file (identifiers +
   the PDF or stored full text) and opening it with StudyAnalyzer; the link
   alone when it has no full text. First full round trip.
4. **iPhone/iPad, then Android**: StudyAnalyzer on each, and MedicalFactChecker's
   sending side. **Starts with a measurement spike**: embedder speed and memory
   on a real phone (model ≈ 1.1 GB at F16, ≈ 0.6 GB at Q8_0; download over
   Wi-Fi with consent). If too slow, the plan decides between Q8_0, embedding
   lazily only when retrieval mode is needed, or both.

Later, outside this design: the further analysis tabs; store submission;
fine-tuning the embedder; other senders (bmlibrarian_lite's desktop app could
write the same hand-off file).

## 10. Facts the builder will need

From surveys on 2026-10-21:

- **MedicalFactChecker for macOS is not sandboxed today** (no app-sandbox key
  in `Sources/MedicalFactCheckerMac.entitlements`); the hand-off design must
  keep working once it is.
- **No app can receive a paper today.** MedicalFactChecker has no URL scheme,
  document types, app group or share extension (iOS/macOS), and only a
  launcher intent filter (Android). Every entry point is new.
- **Where MedicalFactChecker keeps full text:**
  - iOS/macOS SwiftData `Document` (`ios/MedicalFactChecker/Sources/Models/Document.swift`):
    `pmid`, `doi` (`usableDOI`), `pmcId`, `fullTextContent` (markdown),
    `fullTextHTML`, `fullTextPDFPath` + `fullTextPDFPathIsLocalFile`;
    PDFs under `~/Library/Application Support/BioMedLit/PDFCache/`.
  - Android Room `documents`: `pmid`, `doi`, `pmc_id`, `full_text_markdown`,
    `full_text_html`, `pdf_path`; PDFs in `cacheDir/fulltext_pdfs/` (clearable).
- **bmlib Rust** (`~/src/bmlib/rust/bmlib`): synchronous; mature retrieval,
  JATS reader and PDF backend; `LlmClient` with tool use but no streaming; no
  embeddings, vector store, quote matching or PICO. `bmlib-ffi` (UniFFI) is a
  spike consumed by MedicalFactChecker's mobile builds.
- **bmlibrarian's own interrogation tab is a warning, not a template:** it does
  not verify quotes, does not highlight, and embeds the whole conversation as
  the retrieval query.
- **Name clash to avoid:** bmlibrarian_lite has a module
  `study_transparency_analyzer`; it is unrelated.

## 11. Left to the builder

Repository layout and how it depends on bmlib (path vs git dependency until a
release); the hand-off file's container and versioning, and each platform's
wiring within §4.1;
retrieval parameters; the quote-marking format; the find_passage threshold
(from measurement); HTML extraction approach; sqlite-vec or brute force;
how converter-version changes are handled (§5.1); test strategy, following
bmlib's habit of checking ports against the Python as an oracle.
