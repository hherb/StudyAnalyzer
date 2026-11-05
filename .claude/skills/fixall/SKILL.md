---
name: fixall
description: Use when a code review has produced issues that need to be addressed and the pull request finalized on the StudyAnalyzer project (including its bmlib work).
disable-model-invocation: true
allowed-tools: Read, Edit, Bash(git add *), Bash(git commit *), Bash(git push *), Bash(git status *), Bash(git diff *), Bash(git fetch *), Bash(git merge *), Bash(git log *), Bash(git checkout --theirs *), Bash(git checkout --ours *), Bash(gh issue *), Bash(gh pr *), Bash(gh run *), Bash(cargo *), Bash(pnpm *), Bash(uv run pytest *), Bash(ruff *), Bash(uv run ruff *)
---

Address all issues identified in the code review one by one. If fixing them appears manageable within this session, fix them now. If not, lodge the issue on GitHub, on the repository it belongs to. Once all issues have been addressed, run the gates for the code you touched, as listed in CLAUDE.md § Gates. For a bmlib change that means both the Rust and the Python gates, since the two must stay on par.

Then review the code changes thoroughly against CLAUDE.md § Coding rules and the design (docs/superpowers/specs/2026-10-21-studyanalyzer-design.md), and against bmlib's CLAUDE.md and docs/SESSION-RULES.md for bmlib work. If satisfied no issues are left open, update HANDOVER.md and ROADMAP.md ONLY if necessary to reflect these changes (bmlib's own too, for bmlib work). Then commit and push the changes into the PR.

## Leave the PR mergeable

Pushing is not the end. Finish only when the PR is mergeable and its CI is green:

1. **Merge conflicts.** `git fetch origin` and check `gh pr view --json mergeable,mergeStateStatus`. If the base branch has moved on, merge it into the PR branch (`git merge origin/<base>`, no rebase or force-push). Resolve each conflict by understanding both sides: read what the base's new commits did (`git log HEAD..origin/<base>`). When a merged PR already solved the same problem, its approach is the maintainer's decision; keep it and carry over only what this PR still adds on top. Re-run the gates after resolving, then commit the merge and push.
2. **CI.** Watch the PR's checks (`gh pr checks <n> --watch`, or `gh run watch`). When one fails, read its log (`gh run view <id> --log-failed`), find the root cause, fix it locally, run the gates again, push, and watch again.
3. **Repeat** steps 1–2 until `mergeable` is `MERGEABLE` and every check passes. The base can move while CI runs, so check for conflicts again at the end.
4. Never merge the PR yourself, never skip or disable a check, and never weaken a test to make it pass. If a failure is outside this PR (a flaky runner, an outage, a failure that also happens on the base branch), say so with the evidence and stop instead of working around it.

Report the end state: the PR's URL, mergeable, the checks passing, and anything you resolved along the way.
