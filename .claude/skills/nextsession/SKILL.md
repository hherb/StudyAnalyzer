---
name: nextsession
description: Use when starting or resuming a work session on the StudyAnalyzer project, to load current project state and re-establish the coding rules and session workflow before doing any work.
disable-model-invocation: true
allowed-tools: Read, Edit, Bash(git add *), Bash(git commit *), Bash(git push *), Bash(git status *), Bash(git diff *), Bash(gh issue *), Bash(gh pr *), Bash(cargo *), Bash(pnpm *), Bash(uv run pytest *), Bash(ruff *), Bash(uv run ruff *)
---

read HANDOVER.md and ROADMAP.md and follow the instructions. Ask me if you have any questions.

Our coding rules live in CLAUDE.md (§ Coding rules), and the design in docs/superpowers/specs/2026-10-21-studyanalyzer-design.md. Read and honour both. On top of those, follow this session workflow:

1. All gates for the code you touched must pass before committing, unless I explicitly give permission otherwise. The gates are listed in CLAUDE.md § Gates: StudyAnalyzer's own, and bmlib's in **both** Rust and Python for any bmlib change.
2. Before you start working, make sure HANDOVER.md and ROADMAP.md represent the current state of progress and are up to date (check bmlib's merged PRs for phase 1 items). If not, update them before you start.
3. Work on a bmlib item (spec §7) happens in ~/src/bmlib, on its own branch, under bmlib's own HANDOVER.md and docs/SESSION-RULES.md. It lands in Rust and Python in the same PR, with shared contract fixtures. Anything another app could use belongs in bmlib, not here.
4. Avoid technical debt. If you find an error, fix it when you can; otherwise lodge it as a GitHub issue on the repository it belongs to.
5. When you are done, update HANDOVER.md and ROADMAP.md to reflect the current state of development and progress (in bmlib too, for bmlib work). Keep both concise and under 500 lines if possible: move landed items out of ROADMAP.md, focus HANDOVER.md on what still needs doing, and summarise briefly what has already been done. If you are not sure how to do this, ask me.
6. When the task is complete, commit all changes, push, and open a PR to the main branch of the repository you worked in. Link the PR to the relevant GitHub issue if applicable, and include a clear description of the changes made and any relevant context for reviewers. If the repository has no GitHub remote yet, commit and tell me. If you are not sure how to do this, ask me.
