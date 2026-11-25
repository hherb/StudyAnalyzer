# HANDOVER

_Last updated: 2026-11-25 (this session: the Evidence weight tab built on
`feat/evidence-tab`, plan 2 Tasks 1-6. Earlier sessions are in git
history)._

Working notes: what is in flight, what is next and what the maintainer
owes. The calls later work builds on are in `docs/DECISIONS.md`; planned
work not yet started is in `ROADMAP.md`. Each merged phase's full notes are
in git history (this file before 2026-11-14) and in its plan's *Changed
while building* / *As built*.

---

## State

- On GitHub at `hherb/StudyAnalyzer` (public, AGPL-3.0). The design
  (`docs/superpowers/specs/2026-10-21-studyanalyzer-design.md`) is still
  marked *draft for approval*.
- **Phase 1 is done** (every bmlib item of spec §7 and item 11, last merged
  2026-10-31 as bmlib PR #586, in bmlib 0.12.0 / Rust crate 0.3.0).
- **The bmlib pin** (`rev` in the root `Cargo.toml`) is `406315c3`, bmlib PR
  #810's merge (2026-11-22: the `omlx` provider, a local oMLX server at
  `127.0.0.1:8000`, keyless, default model `Inkling-2-Mini-oQ4e-mtp`, its
  window read from `/v1/models`' `max_model_len`; StudyAnalyzer lists it in
  `UNCAPPED_REPLY_PROVIDERS`), past PR #809's merge `b9bfb14b`
  (`suitability_prompt_chars`, counting a retry's correction note), past
  #808 (`PartAssessment.unasked`) and PR #805's merge `a180d330`
  (2026-11-21, the PRISMA 2020 agent), past bmlib 0.14.0 / Rust crate 0.5.0.
  It holds the library steps 1a–1c (#754, #763, #764) and their follow-ups
  through #800, MECA over S3 (#775), PMC's AWS Open Data bucket for JATS and
  the PDF beside it (#749, #752; a md5 mismatch is `checksum-mismatch`) and
  evidence weight phase 1 (#743, for a later *risk of bias* tab). What each
  pin brought, follow-up by follow-up, and what our code does with each:
  `git log -p Cargo.toml` and this file's history (the list stood here until
  2026-11-22).
- A stored PDF text older than `pdf-7` (bmlib #636) or Markdown text older
  than `markdown-2` re-derives on its next open unless a conversation turn
  or a PICO or PRISMA citation points into it (`open::has_dependents`).
- **Phase 2 is merged**: 2a (PR #10), 2b (PR #51), 2c (PR #59), the quote
  guard (PR #62) and 2d (PR #75, fixes in PR #77). Their click-throughs are
  owed (below).
- **The PRISMA 2020 tab** (design `2026-11-18-prisma2020-tab-design.md`,
  plans `2026-11-20-prisma-1-bmlib-agent.md` and `2026-11-20-prisma-2-tab.md`)
  is merged (PR #151, on bmlib PR #805's agent); its hand check is owed
  (below). PR #150 replaced the placeholder app icon for v0.1.0, a version
  never tagged or published. v0.2.0 (PR #158) is the first tagged release,
  a GitHub release with an unsigned macOS `.dmg`.
- **Phase 3 (the shared paper library)**: bmlib's steps 1a–1c and
  StudyAnalyzer's steps 2a, 2b, 2b′ and 2c are merged (PRs #116, #117, #120,
  #123, #125), with follow-ups #127, #128, #131, #133, #135, #136, #141,
  #143 (the clear's users from every record; a folder bmlib refuses asks for
  a rename) and #146 (the re-pin past bmlib #799). Steps 3 (macOS) and 4
  (*Open in StudyAnalyzer*, lite PR #809) are merged in bmlibrarian_lite.
- **The Evidence weight tab** (risk of bias and paper weight): the method is
  `2026-11-09-evidence-weight-design.md` (brought onto main from the unmerged
  branch `docs/evidence-weight-design`, which keeps the RoB 2 smoke test's
  data; the RoB 2 guidance PDF in `docs/smokescreens/private/` is CC BY-NC-ND
  and never committed). `2026-11-24-evidence-weight-tab-design.md` (approved
  2026-11-25) puts the machine verdict first, unattended, with the reader's
  blinded appraisal as a later phase; each run records `first_shown_at` so a
  later appraisal knows it was not blind. Runs are kept in our own tables as
  they were made, never re-judged when bmlib's rules change. Plan 1
  (`2026-11-25-evidence-tab-1-bmlib-steps.md`) is merged as bmlib PR #815
  (`58148480`), with its *Changed while building*; #816's follow-up, bmlib
  PR #817, is merged as `6819565a`.
- **The Evidence weight tab is built on `feat/evidence-tab`** (plan 2,
  `2026-11-25-evidence-tab-2-tab.md`, with its *Changed while building*;
  not yet merged). The pin moved to `6819565a`. Migration 15 and
  `evidence_store.rs` / `evidence_rows.rs` store runs step by step;
  `evidence.rs` drives and resumes them; `evidence_view.rs` serves them;
  `src-tauri/src/evidence_commands.rs` has the commands; the tab is
  `EvidenceTab`, `EvidenceCard`, `EvidenceDomain` and `EvidenceQuotes`
  (`src/lib/evidence.ts`). Its settled calls are in `docs/DECISIONS.md`.
  The reader's own appraisal and later phases are in `ROADMAP.md`.

## Calls later work builds on

The settled decisions of each merged phase (storage, the Interrogator, PICO,
PRISMA, the hand-off, the paper library) are in
[docs/DECISIONS.md](docs/DECISIONS.md). Read it before changing any of them.

## Next

1. **Merge the Evidence weight tab PR** (`feat/evidence-tab`) after its
   review, then phase 3 of the tab from `ROADMAP.md` (the reader's own
   blinded appraisal, compare and conformance).
2. **Deferred from the tab's reviews** (small, none blocks the merge):
   `contract.rs`'s fixture lacks a pending step and a no primary outcome
   run (then assert set equality in `contract.test.ts`); `save_escalation`
   does not check that its escalation agrees with `assessed.rob2`; the
   step-kind strings are repeated in SQL (one constant per kind); a stored
   contradiction's name and D5 measurement positions are not checked;
   `domain_retry_reason` can miss a later failed look (add
   `&& m.second_look_error.is_some()`); `check_escalations`' message for an
   asked escalation with no `escalate`; tuple parameters of `domain_step`
   and `escalation_step`; `EvidenceRunView.study`'s doc ("None until
   assessed": really until the outcomes step adds D1); the busy refusal is
   tested through `with_claim`, not `run_evidence`; `ladderReason` prints
   "null analysed" for a `small_trial` step without `n`.
3. **Phase 3 step 3, MedicalFactChecker (macOS), merged** in
   bmlibrarian_lite: 3a–3d as lite PRs #787, #793, #804 and #805 (the
   clear, on `library_clear.rs`'s rules; its review's two fixes are ported
   back here by PR #143). Their sandboxed click-throughs are owed in
   lite's HANDOVER; leftovers are lite #796, #797, #799, #803. Lite's desktop
   (lite #789) and Android (lite #790) follow, each with its own plan; the
   Android spike is lite PR #807 (merged).
4. **Phase 3 step 4, Open in StudyAnalyzer**, merged as lite PR #809
   (the full-text viewer's button; the paper card's is left for later, the
   maintainer's call). Its click-through is owed in lite's HANDOVER.
5. **What a page cut short should be** (**bmlib #605**, the maintainer's
   call; today a notice).
6. **#31** (two transitive Dependabot alerts) waits on Tauri 3 (glib) and a
   vite-plugin-svelte without deepmerge (both still so on 2026-11-07).
7. **Open issues.** bmlib's are in ROADMAP (*Phase 1*); the nearest are
   **#651** and **#577** (point 4). bmlib **#818** (invalid
   `LadderSettings` in Rust become every outcome's `ladder_error`) and
   **#813** do not touch the Evidence weight tab, which always uses the
   default settings and never reads bmlib's wire form. #686 is in the pin (check the Windows key
   item's naming on Windows). Ours: **#70** (memory at the cap, phase 4),
   **#74** (iOS keyring, a bmlib change for phase 4).
   A Svelte component documents itself in a `/** … */` at the top of its
   script (#14).

## Owed by the maintainer

- **Hand check of the Evidence weight tab** (plan 2 Task 5 Step 5; no agent
  has a window or a key), with `pnpm tauri dev` and a cloud model:
  - [ ] an open-access parallel RCT runs on first open, the status line
    showing each step's label;
  - [ ] *Stop* during outcome 1's D3, then *Continue*, finishes the run;
  - [ ] a citation click scrolls to the passage and highlights it, the
    current one dark, the others pale;
  - [ ] a crossover trial and a cohort study read *No method for this
    design yet*;
  - [ ] at phone width the *Evidence | Paper* switch replaces the split;
  - [ ] a run with a failed D5 second look shows the note beside the number;
  - [ ] open a run after a simulated re-pin (edit its `algorithm_version` in
    SQL): it shows in full with the earlier-rules label and only *Re-run*.
- **Point `/nextsession` and `/fixall` at `docs/DECISIONS.md`** (PR #161's
  review): `.claude/skills/nextsession/SKILL.md` and
  `.claude/skills/fixall/SKILL.md` still name only HANDOVER and ROADMAP, so
  new settled calls would land back in HANDOVER. Left to you, as these are
  the agent's own instructions.

- **Hand check of #149** (needs a real share; CI cannot hang one), with an
  unsandboxed `pnpm tauri dev` and the library on an SMB share: quit, mount
  the share, then stop its server (or pull the network cable of the server
  only); relaunch: the window comes within about 5 s, and the status line
  says the folder *did not answer within 5s … check that it is connected,
  then switch back to the app*, once (not again after *at launch:*). With the app running and the library reachable, stop the server and
  switch to the app: *Choose…* and *No library* in Settings answer within
  about 5 s (before, they hung). Start the server, switch back: the status
  line goes.

- **Click-through of the PRISMA tab** (PR #159; CI never clicks the window):
  - [ ] A run judged from the abstract still says *Judged from its abstract*
    beside its confidence (the field moved into the check).
  - [ ] A failed part names its error; Continue on an earlier incomplete run
    keeps that run shown and read-only, also after pressing Stop.
  - [ ] Switching between a history run and the current run shows the run
    picked last, with the text it was run on.
- **Hand check of oMLX** (PR #158), with oMLX running on `127.0.0.1:8000`:
  - [ ] Settings lists `omlx` with no key field; choosing it offers the
    server's models, `Inkling-2-Mini-oQ4e-mtp` preselected, and shows its
    maximum context window (1,048,576);
  - [ ] an Interrogator question and a PICO run answer, with citations;
  - [ ] with oMLX stopped, asking says the server cannot be reached.
- **Hand check of the curly-quote fallback** (#157), with deepseek-flash
  (which writes `“…”`, not `<q>`):
  - [ ] a quoted sentence from the paper becomes a `[n]` button that
    highlights it; a made-up one keeps its words and is listed as not found;
  - [ ] a quoted term (“per-protocol”) stays plain text, no citation;
  - [ ] while streaming, `“…”` show as plain text and change only when the
    turn completes.
- **Hand check of the PRISMA 2020 tab** (plan 2 Task 5 Step 5; no agent has
  a window or a key), with `pnpm tauri dev` and a cloud model:
  - [ ] open an open-access systematic review with meta-analysis, and one
    RCT;
  - [ ] the RCT shows *not suitable* with its rationale, and *Run anyway*
    runs it (and *Re-run* stays forced);
  - [ ] the review shows the key gaps and the highlights, click → scroll in
    the paper, current one dark, others pale;
  - [ ] *Stop* at part 2, then *Continue* finishes the run (a second
    *Continue* is not offered);
  - [ ] a narrow window (phone width) shows the *PRISMA | Paper* switch;
  - [ ] *Stop* during a *Continue* that retries a failed part: the part is
    still named *failed* with its reason (#151's final check);
  - [ ] a re-check shows *Checking key item … again…* while it runs;
  - [ ] a run whose model left items out (or answered some unasked, or was
    cut off) says so above the notes (*The model left out items …*, #152);
    a small model on a long review is the likeliest to.
- **Look over PR #133's texts** (the cases are hard to arrange by hand;
  the core tests pin the wording): a paper whose JATS cannot be read opens
  from the PDF beside it, with the notice *The document from ‹source› could
  not be read: …. The text is read from the PDF found beside it.*, again on
  a reopen; without that PDF, the *needs full text* reason ends with why.
  After a launch whose lost-bookmark reason was not saved, the status line
  and Settings end the unreachable reason with *; at launch: ‹that reason›*,
  with no older fault (since #134's fix, also *Move into library*, the
  clear's list and a switch's failure entries).
- **Look over PR #131's notices** (they need a database failure, so a
  click-through cannot arrange them; vitest pins the wording): with a
  reachable library and a launch note, the status line reads *The paper
  library: at launch, …* and Settings' caption ends *At launch, …*;
  choosing or clearing the library removes it. A settle failing partway
  reads *N files moved into the paper library, then moving stopped: …*.
- **Click-through of step 2c** (`pnpm dev`, a library holding a few
  papers): *Show the library's papers* lists each with its size and *used
  by this app*; *Remove file* on a PDF this app uses: the question says the
  text and conversations stay, *Keep it* removes nothing, *Remove* deletes
  it from the folder and the list; that paper's Original tab then says the
  file is no longer in the paper library; open the same PDF again: the tab
  works and the file is back in the folder. *Remove paper*: the folder is
  gone in Finder. A paper MedicalFactChecker also uses (once step 3 writes
  records) names it in the question. Delete a paper folder in Finder, then
  *Remove* it from a list shown before: refused, and the list is read again.
  While a removal runs, *Choose…* and *No library* are disabled; unplug the
  library's drive with the list shown, then *Read the list again*: the
  banner says why and the list goes. Since PR #141: rename a paper folder in
  Finder to `con` (or give it a leading space). *Read the list again* still
  lists it, with a `BadFolderName` problem. *Remove paper* and *Remove file*
  on it are both refused with *… Rename it in the file manager, then read
  the list again*, and the folder stays whole. Since PR #143: open that
  paper's Original tab (a reopen shows it only when the text must be
  re-derived) and the reason ends *Rename it in the paper library's folder
  with the file manager, then try again*; paste a new source into that
  paper and its waiting note ends the same way. Put a `records/x.json`
  holding `{` in a paper folder, and its list line reads *other users
  unknown (the scan could not account for every record)* and *Remove paper*
  asks with *… other apps may use it too.* A paper also kept by
  StudyAnalyzer on another Mac names that Mac in the list.
- **Click-through of PR #144** (`pnpm dev`, a library set): rename the
  library folder, paste a new paper (it waits), quit; rename the folder
  back, relaunch: the status line says *1 file moved into the paper
  library.* (before, the launch's report was lost). With Settings open,
  rename the folder away and back and switch to the app: the waiting list
  empties without leaving Settings. On Linux, *Choose…* a symlink to the
  library: Settings shows the real folder, and no paper is listed as left
  elsewhere.
- **Click-through of step 2b′** (a sandboxed build in `/Applications`):
  choose a folder, quit, relaunch: the library is reachable and a library
  PDF's Original tab opens; rename the folder while quit, relaunch: it is
  followed (Settings shows the new path); move it to the Trash while quit,
  relaunch: the status line says it is in the Trash (put it back, relaunch:
  reachable); delete it for good (`rm -r`) while quit, relaunch: the status
  line ends "at launch: …; choose the folder again"; choose another folder:
  the status line goes. *Choose…* a folder inside the Trash: refused. Since
  #126: a library on an SMB share, quit, unmount the share, relaunch: the
  window comes at once (no mount, no password), the status line says the
  share is not connected; connect it in Finder, switch back: the status line
  goes and a library PDF opens. Unplug a USB drive holding it the same way.
  Unsandboxed (`pnpm dev`), #130: put `x` in `library_bookmark` (`sqlite3`),
  relaunch: the library is reachable and the setting holds a new bookmark.
- **Click-through of steps 2a and 2b** (`pnpm dev`, unsandboxed): Settings
  → *Paper library*: a folder holding your own files is refused; *Choose…*
  an empty folder with *Move the N paper files kept in the app into it*
  ticked (*N files moved*, each paper in `papers/…` with a
  `records/studyanalyzer-….json`); open a PMID, a PDF and a paste (each
  lands there); the Original tab of a library PDF; rename the folder: the
  status line appears (on focus), a new paper opens with the *kept in the
  app for now* notice and is in the waiting list; rename it back and switch
  to the app: *1 file moved into the paper library.*; *Choose another…* with
  the copy ticked (the old folder unchanged); *Choose another…* without it
  (*Nothing was copied or moved.*; the Original tab of a PDF left behind
  names the old folder; the setting lists it under *Left in another library
  folder*, and choosing the old folder again empties that list, PR #128);
  *No library* with the copy ticked (`select place, count(*) from sources
  group by place` shows `app`).
- **PR #105's click-through** (done 2026-11-12 but for two items): a PICO
  quote the paper contradicts, listed with its reason (none came up), and a
  5xx or lost network ending in "The fault may be a passing one."
- **Click-through of PR #110**: type `ijms262110631` on the start screen. It
  should say `"ijms262110631" is not a valid PMID, DOI or PMC ID.`, while a
  bad `PMC…` value still names a PMC ID and a bad `10.…/…` one a DOI.

- **Click-through of phase 2b, under way** (2026-11-06): round 1 (local
  model: citations, follow-up, unanswerable question, status line, *Stop*)
  passed; no key is in the database (checked). Round 2 (2026-11-07) so far:
  PICO on `deepseek-flash` (fixed in PR #100, re-run passed) and the model
  menu (lists, choose, *Other…*, no-key note,
  *‹model› (not listed by ‹provider›)*).
  Next in round 2: one PICO run on the cloud model again (bmlib #696's quotes
  beside `reported: false`; #706's 600 s answer wait), a key saved → the
  list, then retrieval mode and the first download (the search model is not
  downloaded yet); one live Anthropic chat (sent no `temperature` since
  bmlib #709) and one live Gemini chat (bmlib #681's documented path, in
  our pin since #732). Keys go through bmlib's store: a key saved before
  still reads; with the keychain unreadable the Settings banner names the
  item and says what helps by kind (#689: damaged → a save replaces it;
  locked → unlock it, no save warning; an ask without a key adds the env
  var).

- **Approve the design**, or note what changes.
- **Decide bmlib #528 and #529** (not hit by plan 2b Task 4's measurement;
  `find_passage`: a superscript digit vs a
  plain one, and a quote whose seeds are all too common). Both touch how
  citations get accepted (§6.3).
- **Click through phase 2a** (plan Task 9 Step 5): open a PDF, a JATS file, a saved
  HTML page, a pasted Markdown text; a PMID with free full text and one
  without; Wi-Fi off for the unreachable path; dark mode. (#7's *Cancel* was
  clicked through on 2026-10-31 and works.)
- **The rest of phase 2b's click-through** (plan Task 10 Step 3), with `pnpm
  dev`, a real cloud model and a local Ollama model, beyond round 1
  (above): *Stop* during *Finding passages* and while a local model reads a
  long prompt (no text yet) (nothing stored, question kept; since bmlib
  PR #714 Ollama's log should show the request ending within about a
  second, not when the prompt is read);
  Wi-Fi off mid-stream and during the download (the offer stays to retry); a
  small context-window setting forcing retrieval (the download with *Cancel*,
  then *Finding passages*); a quote that fails to locate (ask for a
  paraphrase); *New conversation* and an earlier one (*on an earlier version
  of the text*); `needs_model` / `needs_api_key` opening Settings and *← Back*
  keeping the question; *Checking the search model…* and *Finding
  passages…* giving way to *Waiting for the model…* before the first text,
  and *Stopping…* (`applyEvent` is tested, its wiring is not); an unconfirmed text
  showing earlier conversations and the *Confirm above…* note, the box
  appearing on confirm, and no confirm button while an earlier text is shown; a PDF paper's
  *Original* tab; a narrow window for the phone switch; dark mode; quit with the search model loaded
  (no crash report) and reopen (everything still there); the Keychain prompt
  once per launch, not per provider; the key in Keychain Access, not in the
  database (`sqlite3 … "select * from settings"`); and the keychain in a
  sandboxed build.
- **Click through phase 2c** (plan Task 6 Step 3), with `pnpm dev`, a cloud
  model and a local Ollama model: an RCT from PMC (four verified parts; click
  → scroll, current dark, others pale; *Judged from its abstract*); a review
  (not suitable, rationale, *Run anyway*; then *Re-run* stays forced); a long
  paper on a small window (*Condensing the long paper, part n of m…*
  counting up, then *Reading the condensed paper…*; condensed
  caption, quotes still highlight; a question asked meanwhile is answered); *Stop* during condensation (nothing
  stored, *Extract PICO* offered); leaving the tab mid-run and back (the same
  run, no second); a double-click on *Extract PICO* (one run); Wi-Fi off
  during condensation (the error after the retries, the previous run kept);
  *Re-run* with another model (both listed); an earlier run on an older text
  (the banner's confirm held back); no model / no key (a message and a
  button, no jump to Settings); an unconfirmed text, then confirming it with
  the tab open (*Extract PICO* offered); a narrow window (*PICO | Paper*);
  dark mode; quit and reopen (the run shows at once, no new call); the check's
  findings line and *Mean confidence* (#60).
- **Click through phase 2d** (plan Task 7 Step 1; no agent could click the
  window), with a built, sandboxed app in `/Applications` (deep links work
  only there on macOS): choosing and dropping a file still opens and
  attaches; a `.studyanalyzer` file from the fixtures double-clicked with the
  app closed and again with it open (one window), dropped on the window, and
  sent by `open -a StudyAnalyzer file.studyanalyzer`; one double-clicked in
  Finder with the app closed (stored once, opened at mount: the
  early-opened queue); `open "studyanalyzer://open?pmid=…"` with the app
  closed and open; a hand-off while an answer streams and while a PICO run
  goes (it waits, then opens); one arriving while an error banner is shown
  (it waits until dismissed) and one while Settings is open (it waits until
  the user leaves); two in a row (the newest opens, the other is listed); a
  refused file on a cold start (the reason is shown and stays until
  dismissed); the identifiers-only link with no contact email (the email
  screen, then the lookup); quit with a hand-off pending, reopen (it opens);
  Finder shows the file type's name; dark mode. Once Linux or Windows can be
  run (#71): a second launch with a file while the first still starts (stored
  once); a second link while a link-launched app starts (both, once, #94); `studyanalyzer <file> studyanalyzer://open?pmid=…` (both received); `studyanalyzer 'studyanalyzer://[x'` (its refusal shown);
  an AppImage without xdg-utils (the registration dialog).
- **Decide: a hand-off arriving while Settings is open** waits until
  Settings is left (the plan said it would navigate away).
- **Decide bmlib #651**: whether the guard should catch a swap of named
  arms and a spelled-out number (then `MIN_QUOTE_SIMILARITY` can drop
  towards 0.98 and keep dropped-word quotes). #647 (PICO records a refused
  quote's reason) landed in PR #105.
- **Decide** (one line each; the issue holds the argument): **#20** MuPDF's licence and store distribution (blocks
  store submission only); **bmlib #560** how strict `assess_full_text` is
  (moves the shared fixture); **bmlib #561** should Rust's `chat()` retry as
  Python's SDKs do.
- **Decide how large `num_ctx` is on Ollama**: 2b sends the running window,
  often the model's full 131072, so a large local model may fail to load or
  spill to CPU. Sizing it from the prompt estimate could undercount (CJK,
  dense notation) and let Ollama cut the paper silently. Check it in the
  click-through first.
