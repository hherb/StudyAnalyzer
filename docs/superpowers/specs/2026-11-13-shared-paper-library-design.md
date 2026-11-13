# The shared paper library, and phase 3 on it

_Design, 2026-11-13. Draft for approval. It amends the StudyAnalyzer design
(`2026-10-21-studyanalyzer-design.md`) where it says so, and replaces its
phase 3 (§9) with four steps (§8 here)._

## 1. Why

A paper is stored up to three times on one Mac today: in MedicalFactChecker's
PDF cache (inside its sandbox container), in bmlib's full-text cache (under
StudyAnalyzer's own cache directory), and again as a BLOB in StudyAnalyzer's
SQLite (`sources.bytes`). Phase 3 as first designed adds a hand-off copy, and
a paper handed over by identifiers alone is downloaded a second time.

The users are researchers. They keep their sources for years, and the
duplicates grow with the library. One folder of papers that every bmlib app
reads and writes means **each paper is downloaded once and kept once**.

## 2. Goals, non-goals, anti-goals

**Goals**

- One **library folder**, chosen by the user, shared by the bmlib apps:
  StudyAnalyzer and MedicalFactChecker on macOS first, the same design on
  Linux and Windows (StudyAnalyzer builds there already), iOS/iPadOS and
  Android in phase 4, bmlibrarian_lite's desktop app later (through bmlib's
  Python).
- A lookup or fetch in any app looks in the library first and saves what it
  downloads there.
- "Open in StudyAnalyzer" in MedicalFactChecker on macOS (the old phase 3)
  that adds no lasting copy.
- The user can see the folder, back it up, move it, keep it on an external
  or synced drive.

**Non-goals**

- **Users do not add files to the folder.** Only the apps write. A file a
  person drops in is not read (no scanning, no identifying unknown PDFs).
  The "add the PDF you downloaded" route stays unbuilt.
- No reference-manager features: no tags, notes, collections or search over
  the library. Each app's own database keeps what it learns about a paper.
- Not a sync engine. A synced folder (iCloud Drive, Dropbox) is supported by
  keeping every write conflict-free (§4.3), not by resolving conflicts.
- iOS, Android and the desktop Python app are not built now; the design must
  not rule them out.

**Anti-goals** (failure even if it works)

- **A file is deleted or changed without the user asking.** Nothing evicts,
  caps, or expires; no paper file lives where the OS may purge it
  (`~/Library/Caches`). Clearing is always a manual action.
- **A second, silent store.** When the library cannot be reached, an app
  says so. A new source it must keep meanwhile waits in the app's own
  storage, shown as *waiting for the library*, and moves in when the folder
  is back (§4.6); nothing is quietly kept elsewhere for good.
- **A path or URL from a hand-off file is followed.** The trust boundary of
  the StudyAnalyzer design (§4.1) stands unchanged.

## 3. Decided in conversation (2026-11-13)

- A shared library on desktop, rather than copies per app. On phones too,
  where the platform allows a shared user-chosen folder (Android's folder
  picker, iOS's Files).
- **A visible folder the user picks once in each app**, not a hidden shared
  container (App Group). Browsable, backed up and moved like any folder.
- **Nothing is ever deleted automatically.** A manual clear shows which apps
  still use each file.
- **Only the apps write** to the folder.
- **A library that cannot be reached does not stop work** ("open now, move
  later"): a new source is kept in the app's own storage, marked *waiting for
  the library* where the user sees it, and moved in when the folder returns
  (§4.6).
- **What MedicalFactChecker sends** (the old phase 3 question): what
  StudyAnalyzer cannot fetch itself; identifiers alone for a paper whose
  text came from JATS (Europe PMC, PMC's bucket) when MedicalFactChecker has
  no JATS file, else the local PDF, else its Markdown, else its HTML. **When
  the Mac is offline** at the click, whatever MedicalFactChecker has (the
  local PDF, else the Markdown, else the HTML). Once MedicalFactChecker saves
  fetched JATS into the library (§8 step 3), a JATS-derived paper has its
  JATS file and sends that.
- **The writer lives in Swift in BioMedLit** (the shared iOS/macOS package);
  the **library itself is bmlib's, in Rust and Python, reached from Swift
  through bmlib-ffi** (approach A below).

## 4. The library

### 4.1 Choosing it

Each app has a **Library folder** setting: *Choose…* opens the platform's
folder picker. The choice is remembered across launches:

| Platform | How it is remembered |
|---|---|
| macOS (sandboxed) | a security-scoped bookmark (`com.apple.security.files.bookmarks.app-scope`, user-selected **read-write**); access started at launch, stopped at exit |
| iOS / iPadOS (phase 4) | a security-scoped bookmark of a Files folder |
| Android (phase 4) | the folder picker's tree URI with a persisted grant (`ACTION_OPEN_DOCUMENT_TREE`, `takePersistableUriPermission`) |
| Linux, Windows | the path, in the app's settings |

Until a folder is chosen, an app works as it does today (its own storage).
The setting says so and offers *Choose…*; nothing nags.

A chosen folder that is empty becomes a library (the app writes the marker
file, §4.2). A folder holding a library is used as it is. A folder that is
neither (other files, no marker) is refused with a message: a library is
never mixed into a folder of the user's own files, which the apps would then
be tempted to read.

### 4.2 Layout

Plain files only, so a synced folder is safe and a person can read it.

```
<library>/
  bmlib-library.json           {"format": "bmlib-library", "version": 1}
  papers/
    2024 Smith – Metformin and cognition (PMID 38012345)/
      pdf-3f9a1c2e7b40.pdf     a source file, as downloaded or received
      jats-a81d0c55e912.xml
      records/
        studyanalyzer-7c1e….json     what one StudyAnalyzer installation knows and uses
        medicalfactchecker-02ab….json
```

- **A paper folder** has a readable name made, once, from what the creating
  app knows (year, first author, a short title, one identifier). It is never
  renamed: other apps and a sync service may be reading it.
- **A source file** is named by its kind and the first 12 hex digits of its
  SHA-256 (the record holds the whole digest). Its bytes are exactly what was
  downloaded or received: the PDF, the JATS XML, the HTML page, the Markdown
  or text file. Derived text (a canonical text, MedicalFactChecker's
  Markdown converted from JATS) is not a source and stays in the app's own
  database.
- **A record** (`records/<app>-<installation>.json`) is one installation's
  account of the paper. *My call:* per installation, not per app, because
  the same app on two machines sharing a synced folder has two databases and
  would otherwise overwrite one record, losing one machine's uses (and the
  manual clear would then call a used file unused). The installation id is a
  random UUID made at the app's first use of a library, kept in the app's
  settings, and the record carries a label for it (the app and the
  machine's name, *StudyAnalyzer on Horst's MacBook*). The record holds the
  identifiers it knows, a title, and for each file it uses the full SHA-256,
  kind, origin (where it came from, e.g. *Europe PMC*, *hand-off from
  MedicalFactChecker*, *pasted*), size and when it was saved. A file "is
  used by" every installation whose record lists it.

### 4.3 Writing without conflicts

Two apps, or two copies of an app on two machines sharing a synced folder,
can write at once. Every write is made safe by construction:

- **A source file is written once and never changed.** It is written to a
  temporary name and renamed into place (bmlib's `atomic` module). Two apps
  saving the same bytes produce the same name and the same content; the
  second finds it there, checks the digest and is done. A newer copy of a
  paper (a corrected PDF) is a new file beside the old.
- **Each installation writes only its own record.** No file is ever written
  by two writers, so no update is lost and a sync service sees no conflict.
- **A paper is the union of folders whose records share an identifier.**
  Two apps may create two folders for one paper (at the same moment, or one
  knowing only the PMID and the other only the DOI). Readers treat them as
  one paper; nothing moves files to merge them. A folder-name clash with a
  different paper takes a numbered suffix.

### 4.4 Reading

- An app finds a paper by any identifier (canonical forms, as StudyAnalyzer
  stores them today) across all records of all folders.
- **Every read checks the bytes against the record's SHA-256.** A mismatch,
  a missing file, or a file the sync service has not downloaded (an
  online-only placeholder that cannot be fetched offline) is reported as
  such, naming the paper and the file, and is never used.
- An app keeps its own index of the library for speed (it may be rebuilt
  from the folder at any time); the folder is the truth.
- **An app refers to a library file by its SHA-256 and kind, never by a
  path** (*my call*). The digest is the same in every library, so a
  reference survives a move, a switch to another folder, and a device that
  syncs the app's database (MedicalFactChecker's CloudKit) but has another
  library or none: each device resolves the digest in its own library.
- **A file from the library goes through the app's usual intake checks**
  (*my call*): StudyAnalyzer's JATS identifier checks and full-text
  assessment, MedicalFactChecker's PDF title check, whoever saved it. A
  record's account of which paper a file belongs to is another app's claim,
  not proof; downloads are sometimes the wrong article.

### 4.5 Which library file ends a lookup

*My call*, keeping §3's preference for the text StudyAnalyzer reads best:
a lookup uses the library's JATS for the paper when it holds one and asks
the network nothing. When the library holds only weaker kinds (a PDF, HTML,
Markdown, text), the lookup still runs the chain's JATS tiers; JATS found is
saved to the library, and a PDF the chain would download beside it is taken
from the library when present. With no JATS to be had, the library's best
other file is used, in the chain's order (PDF, then HTML, then the rest).

`FullTextCache` keeps what is not a source: the *PDF still owed* mark beside
JATS (bmlib #744), an abstract kept with a PDF, HTML renders. Its JATS and
PDF entries are replaced by the library when one is set. That state is not a
paper, so it may stay in the app's cache directory.

### 4.6 When the library cannot be reached

An unplugged drive, a bookmark that no longer resolves, a revoked grant, a
read-only volume: the app shows it, in the setting and as a banner while it
lasts.

- **A paper whose source is in the library** opens from what the app's own
  database holds (for StudyAnalyzer, its canonical text, conversations and
  PICO runs); what needs the file (the *Original* tab, a re-derive under a
  newer converter) says it is unavailable until the folder is back.
- **A new source** (a lookup's download, a file opened by hand, a paste, a
  hand-off being opened) **opens now** (the maintainer's call, 2026-11-13):
  its bytes are kept in the app's own storage, marked *waiting for the
  library*, and the paper's banner says so. When the folder is reachable
  again (checked at launch and when the app becomes active), the app moves
  every waiting source in as §4.7 moves a file (copy, check the digest,
  record, then drop its own copy), and says how many it moved. A failure
  leaves the waiting copy and names the paper. The list of waiting sources
  is in the setting.

### 4.7 Moving existing papers in

When a folder is first chosen, the app offers to move what it already keeps
into it (StudyAnalyzer: every `sources` row; MedicalFactChecker: its PDF
cache). Each file is copied, its digest checked in place, recorded, and only
then dropped from the app's own storage. It is the user's choice, never
automatic, and can be run later from the setting. A failure leaves the
app's own copy and says which paper.

**Choosing another folder, or none** (*my call*). References are digests
(§4.4), so after a switch each is looked up in the new library. The switch
offers to copy every file this installation uses from the old folder into
the new one (when the old one is still reachable); declined or failed, a
file missing from the new library reads as unavailable (§4.6), naming it.
Going back to *no library* offers to copy the files this installation uses
back into the app's own storage; declined, they read as unavailable. The
old folder is never changed by a switch: other apps may still use it.

### 4.8 The manual clear

From each app's setting: a list of the library's papers with, for each, the
apps that use it and its size. The user removes a paper (its folder) or a
single file, after a confirmation that names the other apps still using it.
An app that later finds a file gone reports it (§4.6); it does not
re-download on its own.

## 5. What changes in each app

### 5.1 bmlib (Rust and Python, the same PR, shared fixtures)

A `library` module: open or create a library at a path; find a paper by
identifiers; save a source file (bytes, kind, origin, identifiers, title) →
its reference (paper folder, file name, SHA-256); read a file by reference,
digest checked; update this app's record (add identifiers, list a file as
used, drop it); list papers with their uses (the manual clear); remove a
paper or a file. Errors are typed: not a library, unreachable, digest
mismatch, missing, not downloaded, read-only.

Its file operations sit behind one small module, so phase 4's Android, whose
folder grant gives content URIs rather than paths, can supply them from
Kotlin through bmlib-ffi without a rewrite.

`FullTextService` gains the library as its cache: with a library set, a
lookup looks there first by identifier and saves JATS and PDFs there, in
place of `FullTextCache`. Without one, `FullTextCache` as today.

`bmlib-ffi` exposes the module to Swift (and later Kotlin). It needs neither
`sqlite` nor `http-ureq`, which the FFI build leaves out.

Contract fixtures: a small library tree (two apps' records, a split paper,
a digest mismatch, a missing file) and `cases.json` of finds, saves and
reads with their outcomes, run by the Rust tests, the Python tests and
(through the FFI) BioMedLit's tests.

### 5.2 StudyAnalyzer

- **Amends design §4.1 and CLAUDE.md's "received full text is copied into
  app storage immediately"**: with a library set, a received or fetched
  source is saved **into the library** immediately (still before the
  granted access may end) and `sources` stores a reference (folder, file,
  SHA-256) in place of the bytes. Without a library, the BLOB as today.
- Migration: `sources.bytes` becomes nullable, with `sha256` beside it and
  a state: *in the app* (bytes, no library), *waiting for the library*
  (bytes, to be moved), *in the library* (no bytes; the digest is the
  reference, §4.4). `holder_of_source` compares digests rather than bytes.
- **A received hand-off waits in `handoffs` as today** (*my call*): its
  bytes stay in that table until the user opens or discards it. That queue
  is transient, like MedicalFactChecker's temporary file; opening goes
  through intake as below, and a discard writes nothing to the library. `original_pdf`, a re-derive and the
  PDF beside JATS read through the library, digest checked.
- Every source it stores goes to the library: lookups (through
  `FullTextService`), files opened by hand, pastes (as a text file) and
  hand-offs.
- **A hand-off's bytes**: StudyAnalyzer hashes them. If the library holds a
  file with that digest, the source references it and nothing new is
  written; otherwise the bytes are saved into the library once. **The
  hand-off format does not change**, and nothing in a hand-off file is
  followed.
- Entitlements: `files.user-selected.read-write` (from read-only) and
  `files.bookmarks.app-scope`. Bookmark creation and resolution from Rust on
  macOS (through `objc2-foundation`, MIT/Apache, or the builder's choice of
  an AGPL-compatible binding).
- The Library setting, the move (§4.7) and the manual clear (§4.8) in
  Settings; the unreachable state (§4.6) in the paper banner.
- `open::has_dependents` and the converter rules are unchanged: they are
  about the canonical text, which stays in SQLite.

### 5.3 MedicalFactChecker (macOS now; the iOS half in phase 4)

- The Library setting (Settings), the move of its PDF cache (§4.7), the
  manual clear (§4.8).
- Its full-text fetch (`FullTextService` in BioMedLit, its own Swift chain,
  not bmlib's) looks in the library first by identifier and saves what it
  downloads there: PDFs, and from now on **the JATS XML it converts**, which
  it does not keep today. `Document` gains the digest and kind of its
  library files (§4.4), which CloudKit may sync: another device resolves
  them in its own library, or reads them as not in its library and fetches
  as today. Its Markdown and HTML stay in SwiftData. A source fetched while
  the library cannot be reached waits in its PDF cache, marked (§4.6).
- `com.apple.security.files.bookmarks.app-scope` added (it has user-selected
  read-write already).
- **Note on spec §10:** MedicalFactChecker for macOS **is sandboxed now**
  (`com.apple.security.app-sandbox` in `MedicalFactCheckerMac.entitlements`,
  checked 2026-11-13); §10's "not sandboxed today" is out of date.

### 5.4 Open in StudyAnalyzer (the old phase 3)

- `StudyAnalyzerHandoff` in BioMedLit (pure Swift): builds the
  `.studyanalyzer` JSON for a `Document` by §3's rule (what StudyAnalyzer
  cannot fetch itself; when offline, whatever it has), checked against
  StudyAnalyzer's hand-off fixtures (`docs/handoff-format.md`). **Always a
  file, never the `studyanalyzer://` link**, even for identifiers alone: a
  file carries the title and the sender's name, and on macOS a link works
  only once StudyAnalyzer is in `/Applications`. *Amends design §9 phase 3*
  ("the link alone when it has no full text").
- The macOS card's full-text row and context menu, and the full-text
  viewer's toolbar, get **Open in StudyAnalyzer**. It is shown only when
  StudyAnalyzer is installed
  (`NSWorkspace.urlForApplication(withBundleIdentifier: "com.bmlibrarian.studyanalyzer")`).
- The file is written in MedicalFactChecker's temporary directory and
  opened with `NSWorkspace.open(_:withApplicationAt:configuration:)`; Launch
  Services grants StudyAnalyzer that one file. It is a transient copy:
  StudyAnalyzer holds it in its hand-off queue until opened or discarded,
  then keeps nothing of it beyond the library file (§5.2).
  MedicalFactChecker deletes its earlier hand-off files on the next send and
  at launch.
- "Offline" is the current path's status from `NWPathMonitor` at the click.
- A failure (StudyAnalyzer gone, the open refused, the file not written) is
  shown on the card, never silently.

## 6. Approaches considered

Where the shared code lives (the store and MedicalFactChecker's side of it):

- **A. One implementation in bmlib (Rust, with its Python port), reached
  from Swift through bmlib-ffi.** **Chosen.** MedicalFactChecker already
  ships bmlib-ffi (its retraction checks). One set of rules for the code
  that guards a researcher's papers for years; Swift resolves the bookmark
  and hands Rust a path. Cost: every library change rebuilds the
  xcframework and moves MedicalFactChecker's pin (`BmlibFFI.pin.json`).
- **B. A Swift implementation in BioMedLit against bmlib's fixtures.** No
  bridge to rebuild, native file APIs; but the layout, the atomic writes,
  the records and the move would exist in Rust, Python and Swift, and in
  Kotlin later, every change landing in each. Rejected for that drift.
- **Hidden shared container (App Group) instead of a visible folder.** No
  prompt, but the library is buried in `~/Library/Group Containers`, cannot
  be moved or backed up on purpose, and Android has no equivalent. Rejected
  in conversation.
- **The hand-off carries a reference into the library instead of the
  bytes.** No transient copy, but it fails whenever the two apps' folders
  differ or one has none, and a reference is a path-like thing inside a
  hand-off file. The bytes in a transient file cost nothing lasting;
  StudyAnalyzer dedupes them by digest. Rejected.

## 7. Constraints

- CLAUDE.md's coding rules: every outward function documented and tested,
  no magic numbers, every error handled, logged and shown, nothing truncated,
  AGPL-compatible dependencies only.
- bmlib's rule: Rust and Python on par in the same PR, with shared contract
  fixtures; its own HANDOVER and SESSION-RULES.
- Sandboxed on macOS throughout (both apps); built App-Store-ready.
- File names valid on macOS (APFS, NFD-normalised by Finder), Windows (no
  `<>:"/\|?*`, no trailing dot or space, reserved names, 255-character
  components) and Linux; within synced-folder limits.

## 8. Order of work

Four PRs, each with its own implementation plan in `docs/superpowers/plans/`
(the first two in this repository's plans, as the phase 2 plans are;
MedicalFactChecker's in bmlibrarian_lite):

1. **bmlib**: the `library` module in Rust and Python, `FullTextService`'s
   use of it, the bmlib-ffi surface, the fixtures.
2. **StudyAnalyzer**: the setting and bookmark, sources by reference, the
   move, the manual clear, the unreachable state, hand-off bytes deduped.
3. **MedicalFactChecker (macOS)**: the xcframework re-pinned, the setting
   and bookmark, its fetch through the library (JATS kept), the move, the
   manual clear.
4. **Open in StudyAnalyzer**: `StudyAnalyzerHandoff`, the buttons, the
   transient file. The first full round trip.

## 9. Left to the builder

- The exact folder-name rule (which fields, the length, the character
  substitutions per platform, the numbered suffix), within §7.
- The records' JSON shape and the library marker's fields, versioned like
  the hand-off format (an optional field does not bump `version`).
- Each app's index of the library (what it caches, when it rescans).
- How the manual clear and the move look in each app's Settings.
- Whether StudyAnalyzer's Linux and Windows builds get the setting in PR 2
  or later (the design holds there; the bookmark is macOS-only code).
