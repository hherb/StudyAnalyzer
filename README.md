# StudyAnalyzer

**Read one biomedical paper closely, and question it with an AI model of your
choice. Every answer points to the passage it comes from.**

> **Status: early development.** The app opens a paper from a file, pasted
> text or an identifier, finds its full text and shows it. The Interrogator
> and PICO tabs are not built yet, and there is nothing to download. The
> design is in
> [`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`](docs/superpowers/specs/2026-10-21-studyanalyzer-design.md).
> See [ROADMAP.md](ROADMAP.md) for what is planned and [HANDOVER.md](HANDOVER.md)
> for where work stands.

## What it will do

An abstract often does not say what you need to know about a study: who
exactly was in it, what was compared, or whether there is anything about
patients over 75. StudyAnalyzer takes the **full text** of one paper and lets
you ask.

- **Interrogator.** Chat about the paper, with the paper beside the chat.
  Every claim in an answer cites a quote, and the quote is checked against the
  paper before it is shown. Clicking a citation scrolls to the passage and
  highlights it. Passages cited earlier in the conversation stay highlighted
  more faintly, so you can see everything the conversation relied on.
- **Honest about what it could not find.** It never answers from the abstract
  or from the model's general knowledge while appearing to answer from the
  paper. A quote that cannot be found in the paper loses its citation, and the
  answer says so.
- **PICO.** A ready-made breakdown of population, intervention, comparison and
  outcome, plus study type and sample size. Each part is backed by a cited,
  verified passage. Further analyses will follow, each in its own tab.
- **Gets the full text for you.** It uses the text a sending app passed along.
  Otherwise it retrieves the paper from open sources (Europe PMC, PubMed
  Central, Unpaywall and others). Failing that, you can open a PDF, JATS XML,
  HTML, Markdown or text file, or paste the text.
- **Remembers everything.** Each paper, its text and your conversations are
  stored locally, so reopening a paper is instant and works offline.
- **Your model, your choice.** Anthropic, OpenAI-compatible endpoints or a
  local model through Ollama. API keys are kept in the operating system's
  keychain. In the maintainer's experience, models under about 20B parameters
  do this job poorly.

## How it fits in

StudyAnalyzer is a companion to the other apps in the
[bmlib](https://github.com/hherb/bmlib) family, starting with
MedicalFactChecker. There you can open a paper in StudyAnalyzer from the paper
card or the full-text viewer. It also works on its own: open a file, paste
text, or enter a PMID, DOI or PMC ID.

It is written in Rust with a [Tauri 2](https://tauri.app) interface (Svelte 5
and TypeScript). Shared logic, such as full-text retrieval, quote matching,
embeddings and the PICO analysis, lives in bmlib, so other apps get it too.

## Platforms

macOS first, then Linux and Windows, then iPhone and iPad, then Android. The
plan is to distribute through the Apple App Store and Google Play eventually.
It will always be buildable from source here.

## Building from source

You need Rust (stable), Node 22+ and pnpm, plus the platform prerequisites
for [Tauri 2](https://tauri.app/start/prerequisites/). The first build
compiles MuPDF from source for bmlib's PDF support (a C toolchain with make,
or MSBuild on Windows, and libclang).

```sh
pnpm install
pnpm dev            # run the app
pnpm build          # build a bundle
```

Before a commit, run the checks CI runs:

```sh
pnpm run check:web && pnpm run test:web && pnpm exec vite build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The logic lives in `crates/studyanalyzer-core` (no Tauri needed to test it),
`src-tauri` holds the commands, and `src/` the Svelte interface. bmlib is a
git dependency pinned by commit in the root `Cargo.toml`.

## Licence

[GNU Affero General Public License v3.0](LICENSE). Third-party components
(llama.cpp, MuPDF, the snowflake-arctic-embed embedding model) keep their own
licences, and their notices will ship with the app. MuPDF's copyright is
Artifex's, so app-store builds wait on its licence (issue #20).
