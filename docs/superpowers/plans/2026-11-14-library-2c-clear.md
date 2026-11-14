# Shared paper library, step 2c: the manual clear

**Goal:** From the Library setting, the user sees the library's papers (each
with its size, its files and the apps that use them) and removes a paper's
folder or one file, after a confirmation naming the other apps still using
it (design `2026-11-13-shared-paper-library-design.md`, §4.8). Nothing is
removed otherwise.

**Spec:** §4.8, §5.2 (the clear in Settings), §9 (how it looks is the
builder's).

## Calls (the builder's, within §4.8 and §9)

- **On request.** *Show the library's papers* scans once; *Read the list
  again* rescans. A large library is not scanned on every visit to Settings.
- **One entry per paper folder.** Folders that form one paper (§4.3, records
  sharing an identifier) are listed apart. shortcut: group them with
  `bmlib::library::group` if split papers turn up in practice.
- **Per file:** its name (kind and digest, as Finder shows it), its size from
  the folder (decimal units, as Finder counts) and the installations whose
  records list it, this one shown as *this app*. A file no record lists is
  listed as used by no app and can be removed.
- **The scan's problems** (an unreadable record, a file no record lists…) are
  logged and listed under the papers as bmlib's `Debug` text, free-form (the
  maintainer's rule: surface diagnostics while developing).
- **The confirmation** is in the page (the webview shows no dialogs). It
  names every other installation using the paper or file and, when this one
  uses it, says what stays: the text, conversations and PICO runs, and that
  opening the same file again puts it back.
- **Removal re-scans first** and refuses a folder or file the scan no longer
  lists (a stale list, or a name the webview made up), removing nothing;
  bmlib refuses a name that is not one path component. Listing and removal
  hold `library_op`, so neither races an open or a change of the setting.
- **After a removal** this installation's rows stay `library`; a read says
  the file is no longer in the paper library (§4.6). Other installations'
  records keep listing a removed file, as bmlib's `remove_file` leaves them.

## Found while building

- **Reopening a removed file stored nothing.** The same document again is
  `StoredCopy::Same`, which wrote no bytes, so a file the library lost could
  never return. `open::persist` now brings the bytes back (`bring_back`, as
  waiting or in the app) whenever the stored copy is in the library, and
  the open's settle saves them again, or finds the file still there.
- **A removed JATS blocked settling for good.** `library::foreign_jats` read
  every JATS a record lists; one removed (by this clear, or by another app's)
  is listed but not on disk, so the read failed and the source waited for
  ever. A listed file that is not on disk is now passed over.

## Shape

- Core: `library_clear.rs` (`list`, `remove`; `Listing`, `ListedPaper`,
  `ListedFile`), `library_setting::ready` (shared with `move_in`) and
  `SettingError::Library`.
- Tauri: `library_papers`, `remove_library_paper(folder)`,
  `remove_library_file(folder, name)`, in the contract.
- UI: `LibraryPapers.svelte` under the setting while a usable library is set
  (keyed on the folder); wording in `src/lib/library.ts`.
