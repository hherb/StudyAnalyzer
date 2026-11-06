---
name: finalcheck
description: final pre-merge pass on the current StudyAnalyzer PR — review, fix everything, then drive GitHub CI to green
disable-model-invocation: false
allowed-tools: Bash(git *), Bash(gh *), Bash(cargo *), Bash(pnpm *), Bash(uv *), Bash(uvx *), Bash(bash *)
---
Run these steps in order on the PR for the current branch. Sessions here can work in `git worktree`s
under `.claude/worktrees/`, so first run `pwd && git branch --show-current && git worktree list` and `cd`
into the PR's worktree. Then find the PR with `gh pr view --json number,url,headRefOid`. If there is
none, stop and tell me. A bmlib PR (`~/src/bmlib` or a sibling `../bmlib-<topic>` worktree) gets bmlib's
own `finalcheck`, not this one.

1. **Review.** Invoke the `pr-review-toolkit:review-pr` skill via the Skill tool and let it finish.
2. **Fix.** Merge `origin/main` into the branch first (`git fetch origin && git merge origin/main`;
   resolve conflicts, never rebase a pushed branch). Then apply the `fixall` skill to that review's
   findings. It is not model-invocable, so read `.claude/skills/fixall/SKILL.md` and follow it. It ends
   by committing and pushing into the PR. Before moving on, confirm the push landed:
   `git fetch origin && git status -sb` must show no `ahead`, and the PR's `headRefOid` must equal
   `git rev-parse HEAD`. Findings deferred to an issue go in the PR body and `HANDOVER.md`, and a
   closing keyword is never written next to an issue number the PR does not close.
3. **Drive CI to green.** Loop:
   - Watch the checks for the pushed head commit with `gh pr checks --watch --fail-fast`. Checks can take a
     minute to register, so wait if it reports none yet. CI is one job, `verify` (`.github/workflows/ci.yml`),
     on every PR with no path gate, so a missing check is stuck, not normal.
   - All green → done. Report the PR URL and the final check list. "All green" means `verify` passed.
     `main` has no ruleset or branch protection (`gh api repos/hherb/StudyAnalyzer/rulesets` is `[]`), so
     `verify` is the whole bar.
   - A check failed → read the cause: `gh run list --branch "$(git branch --show-current)" --limit 5`, then
     `gh run view <run-id> --log-failed`. Reproduce it locally with the command the failing step runs,
     from the repo root (these are CLAUDE.md § Gates):
     - `Install JavaScript dependencies` → `pnpm install --frozen-lockfile` (a lockfile out of step with
       `package.json`)
     - `Type-check the interface` → `pnpm run check:web`
     - `Test the interface` → `pnpm run test:web`
     - `Build the interface` → `pnpm exec vite build`
     - `Check formatting` → `cargo fmt --all --check`
     - `Lint the Rust` → `cargo clippy --workspace --all-targets -- -D warnings`
     - `Test the Rust` → `cargo test --workspace`. A contract failure (`json_matches_the_fixture`) means
       the frontend's JSON changed: rerun with `UPDATE_CONTRACT=1`, review the fixture's diff, and update
       `src/lib/api.ts` to match.
     - A bmlib fetch or compile failure → check the `rev` pin in the root `Cargo.toml` is a commit on
       bmlib's `main` (`git -C ~/src/bmlib branch -r --contains <rev>`).

     Judge each command by its **exit code**, never by filtered output: `cmd | tail` reports `tail`'s
     status (use `set -o pipefail`). Fix the root cause, using TDD where it is a code bug, and check the
     design (`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`) and the plan's *Changed while
     building* before "fixing" anything that looks deliberate. Confirm the fix passes locally, commit with a
     message naming the CI failure, push, and loop again.
   - CI runs on `macos-latest`. A failure that passes locally is most often a toolchain difference
     (`dtolnay/rust-toolchain@stable` vs your `rustc`, a newer clippy lint, Node 22 and pnpm 11):
     `rustup update stable` and match the versions before guessing.
   - **A job that is CANCELLED with no log** was killed, not slow (the job has a 45-minute timeout). Find
     what killed it. Do not rerun it as a flake.
   - If the failure is not caused by the code (runner/infra outage, network flake, a crates.io, npm or
     GitHub fetch), rerun once with `gh run rerun <run-id> --failed`. If it fails the same way again, stop
     and report rather than changing code to work around it. The Actions API returns empty or stale data
     during an outage, so re-query before concluding a run never happened.
   - Never "fix" CI by skipping, `#[ignore]`-ing, `.skip`-ping or deleting tests, by weakening lint or CI
     config, guards or thresholds (`MIN_QUOTE_SIMILARITY`, size caps), or by adding an `#[allow]` or
     `@ts-ignore` without the reason at the site, without my explicit permission. Never merge the PR
     yourself. If the same failure survives three fix attempts, stop and report what you tried.
4. **Not covered by CI.** CI never clicks the window. If the PR changes what the UI shows, list what a
   click-through should check in the report (and in `HANDOVER.md`'s *Owed by the maintainer*).
