# HANDOVER

_Last updated: 2026-10-27._

Working notes for picking up in-flight work. Each section is one self-contained
slice: what's known, where to start, and how to verify. Remove a section once
its slice has landed; add a new section when handing off new work. Planned
work not yet started is in `ROADMAP.md`.

---

## State

- **This repository holds the design only**: no code yet. It is on GitHub
  at `hherb/StudyAnalyzer` (public, AGPL-3.0); `main` holds only the initial
  commit until **PR #1** (design, session docs, README, CI scaffold) merges. CI (`.github/workflows/ci.yml`)
  is a scaffold whose steps start running once `Cargo.toml` / `package.json`
  exist. The design (`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`) is
  marked *draft for approval*.
- **Phase 1 runs in bmlib** (`~/src/bmlib`). bmlib keeps its own HANDOVER,
  ROADMAP and CHANGELOG for that work, and its `docs/SESSION-RULES.md` apply
  there. This file only tracks where phase 1 stands.

## In flight

**Item 9, retrieval returns its source**: bmlib **PR #545** (branch
`feat/fulltext-jats-source`, worktree `~/src/bmlib-jats-source`), all gates
green in both languages, awaiting review and merge. `FullTextResult.jats_xml`
holds the JATS bytes as served; the cache keeps them in `jats/`. A PDF stays
at `file_path`. The maintainer's calls (2026-10-27): raw bytes; an old HTML-only cache
entry is served with `jats_xml=None`, not re-fetched; no `pdf_bytes`. So
**StudyAnalyzer must give `FullTextService` its own cache directory** in app
storage (the sandbox needs that anyway), and treat `jats_xml=None` on a
`fulltext` result as "no structured source".

## Next

**The next phase 1 item.** In the order of `ROADMAP.md`: **item 4**
(canonical text, which reads item 9's `jats_xml`), then **3** (the chunker
points into 4's text). Item 4 can branch off #545 if it has not merged yet.
**Item 6** (streaming) is independent and can go at any point.

Start each item in bmlib: read bmlib's `HANDOVER.md` and
`docs/SESSION-RULES.md`, write the item's plan in
`~/src/bmlib/docs/superpowers/plans/`, and branch off bmlib's `main`. Verify
with bmlib's gates in both languages (`CLAUDE.md` § Gates). When the PR
merges, move the item out of `ROADMAP.md` and into *Landed* below.

## Owed by the maintainer

- **Approve the design**, or note what changes.
- **Decide bmlib #528 and #529** (`find_passage`: a superscript digit vs a
  plain one, and a quote whose seeds are all too common). Both touch how
  citations get accepted (§6.3).

## Landed (context)

Phase 1 items merged in bmlib, all **unreleased** (bmlib 0.11.0 predates them):

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
