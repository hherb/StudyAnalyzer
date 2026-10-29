# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## State of the repository

Phase 2a's code exists: a root Cargo workspace with `crates/studyanalyzer-core`
(all logic, testable without Tauri: identifiers, intake, storage, retrieval,
opening a paper, the UTF-16 view) and `src-tauri` (thin commands), and the
Svelte frontend in `src/`. bmlib is a git dependency pinned by `rev` in the
root `Cargo.toml`. The source of truth is
[docs/superpowers/specs/2026-10-21-studyanalyzer-design.md](docs/superpowers/specs/2026-10-21-studyanalyzer-design.md);
read it before any work. Items marked *decided* or *my call* are settled; §11
lists what is left to the builder. Each phase (§9) gets its own implementation
plan in `docs/superpowers/plans/`; phase 2 is four (2a–2d). Phase 1 is bmlib
work, not code in this repository.

Progress lives in [HANDOVER.md](HANDOVER.md) (in flight, next, owed) and
[ROADMAP.md](ROADMAP.md) (planned, not started). Start a session with
`/nextsession` and finish a review with `/fixall`.

## Gates

Everything must pass before a commit.

- **bmlib, Python** (from `~/src/bmlib`): `uv run pytest tests/ -v`,
  `ruff check .`, `ruff format --check .`
- **bmlib, Rust** (from `~/src/bmlib/rust`): `cargo test`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo fmt --check`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features`.
  Single test: `cargo test <name>`. In a sandbox that denies `~/.cargo`, prefix
  `CARGO_HOME="$PWD/.cargo-home"`.
- **StudyAnalyzer** (from the repo root; CI runs the same):
  `pnpm run check:web`, `pnpm run test:web`, `pnpm exec vite build`,
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`. A change to these commands changes
  [ci.yml](.github/workflows/ci.yml) with it.

## Coding rules

The family's house rules (from bmlibrarian_lite's `doc/llm/general_golden_rules.md`):

1. Clean separation of concerns between modules.
2. Prefer reusable pure functions over complex classes.
3. Every function, type and method has a doc comment a junior developer can follow.
4. No magic numbers: constants and settings live in a settings/config module.
5. Every outward-facing function, type and method has unit tests.
6. Never truncate data unless the user explicitly asks. Never introduce truncation without the maintainer's approval.
7. Network, LLM and API calls that can fail or time out retry with exponential backoff.
8. Every error is handled, logged and reported to the user. No exceptions.
9. Read the documentation of a less familiar library or API before using it.
10. Keep cross-platform compatibility in mind (desktop and mobile here).
11. AGPL-3.0: no incompatible third-party libraries.

## What it is

A companion app for reading one biomedical paper closely. Another app (first
MedicalFactChecker) hands it a paper. It fetches the full text and offers an
**Interrogator** tab (chat with verified, highlighted citations) and a **PICO**
tab, the pattern every later analysis tab follows.

Stack: **Rust + Tauri 2**, frontend **Svelte 5 + TypeScript + Vite** (as in
`~/src/hanzitutor`, `~/src/bottie`). Licence AGPL-3.0. macOS first, then
Linux/Windows, iPhone/iPad, Android.

## Rules that shape the code

- **bmlib first.** Anything another app could use is built in bmlib
  (`~/src/bmlib`, Rust in `rust/bmlib`), not here. Every bmlib addition lands
  in **Rust and Python on par, in the same PR**, held together by shared
  contract fixtures (the precedent is bmlib#504). Spec §7 lists the pieces.
- **Port from bmlibrarian** (`~/src/bmlibrarian`), using its Python tests as
  the oracle: `utils/passage_match.py` (`find_passage`) and `agents/pico_agent.py`.
  bmlibrarian's own interrogation tab is a warning, not a template.
- **One canonical text per paper** (title + ordered sections). All offsets
  (chunks, citations, highlights) point into it, carry its converter version,
  and use one unit. The code-point ↔ UTF-16 conversion happens only at the
  Rust/frontend boundary.
- **Citations:** every quote is located by `find_passage`. The paper's own span
  is what gets highlighted and stored, never the model's copy. A quote that
  cannot be located loses its citation, with a visible note, never silently.
- **Never fall back to the abstract** or to the model's general knowledge.
  Without full text the Interrogator does not open.
- **Sandbox-ready from the start** (App Store, Play). No file paths are passed
  between apps: hand-off is a `.studyanalyzer` file or a `studyanalyzer://`
  link (identifiers only). Received full text is copied into app storage
  immediately.
- **Trust boundary:** validate every identifier's format, cap the hand-off file
  size, and never dereference paths or URLs found inside it.
- **Persist everything** about a paper in SQLite (via bmlib's `db` layer).
  Nothing learned about a document lives only in memory.
- API keys go in the OS keychain, with env vars as fallback. Never in the DB
  or plain files. No model is hard-wired.

## UI reference

Mockups in [docs/superpowers/specs/mockups/](docs/superpowers/specs/mockups/):
`interrogator-layout-v2.html` (layout), `visual-style.html` (style A
"Journal"). Chat on the left, paper on the right. The current citation is
highlighted `#e8b931`, earlier ones `#f6e7b0`. On phones a *Chat | Paper*
switch replaces the split.
