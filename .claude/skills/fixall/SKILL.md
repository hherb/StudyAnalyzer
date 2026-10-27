---
name: fixall
description: Use when a code review has produced issues that need to be addressed and the pull request finalized on the StudyAnalyzer project (including its bmlib work).
disable-model-invocation: true
allowed-tools: Read, Edit, Bash(git add *), Bash(git commit *), Bash(git push *), Bash(git status *), Bash(git diff *), Bash(gh issue *), Bash(gh pr *), Bash(cargo *), Bash(pnpm *), Bash(uv run pytest *), Bash(ruff *), Bash(uv run ruff *)
---

Address all issues identified in the code review one by one. If fixing them appears manageable within this session, fix them now. If not, lodge the issue on GitHub, on the repository it belongs to. Once all issues have been addressed, run the gates for the code you touched, as listed in CLAUDE.md § Gates. For a bmlib change that means both the Rust and the Python gates, since the two must stay on par.

Then review the code changes thoroughly against CLAUDE.md § Coding rules and the design (docs/superpowers/specs/2026-10-21-studyanalyzer-design.md), and against bmlib's CLAUDE.md and docs/SESSION-RULES.md for bmlib work. If satisfied no issues are left open, update HANDOVER.md and ROADMAP.md ONLY if necessary to reflect these changes (bmlib's own too, for bmlib work). Then commit and push the changes into the PR.
