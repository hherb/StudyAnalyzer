<script lang="ts">
  /**
   * The app's root: holds the state, runs the commands and shows one screen at a time
   * (start, needs full text, paper, settings), with the error banner above.
   */
  import { onMount, untrack } from "svelte";
  import ErrorBanner from "./components/ErrorBanner.svelte";
  import FullTextBanner from "./components/FullTextBanner.svelte";
  import Interrogator from "./components/Interrogator.svelte";
  import NeedsFullText from "./components/NeedsFullText.svelte";
  import Settings from "./components/Settings.svelte";
  import StartScreen from "./components/StartScreen.svelte";
  import * as api from "./lib/api";
  import { LOOKING, OPENING, handoffBusyText, nextHandoff } from "./lib/handoff";
  import { bannerNotes, bannerText, retryIdentifier } from "./lib/paper";

  const CANCELLING = "Cancelling…";

  /** Which screen shows. */
  type Screen = "start" | "needs_full_text" | "paper" | "settings";
  let screen = $state<Screen>("start");
  /** The screen Settings returns to. */
  let previous: Screen = "start";
  let error = $state<string | null>(null);
  let busyText = $state<string | null>(null);
  let busy = $derived(busyText !== null);
  let recent = $state<api.PaperSummary[]>([]);
  let view = $state<api.PaperView | null>(null);
  /** What the open that produced `view` asked the user to know (Rust's notices). */
  let notices = $state<string[]>([]);
  let missing = $state<{ paperId: string; reason: string; unreachable: boolean } | null>(null);
  let email = $state("");
  /** The debugging setting: a mismatching document opens as a paper of its own; `null` until read. */
  let keepMismatched = $state<boolean | null>(null);
  /** What stopped for want of a contact email (a lookup or a hand-off); re-run once it is saved. */
  let pendingRetry: (() => Promise<void>) | null = null;
  /** The hand-offs waiting, newest first. */
  let pending = $state<api.HandoffSummary[]>([]);
  /** Hand-offs already tried this launch, so a kept one is not retried in a loop. */
  const tried = new Set<number>();
  /** The Interrogator's question box, kept here so it outlives a trip to Settings. */
  let question = $state("");
  /** Whether the Interrogator is answering, downloading or loading (navigation waits). */
  let interrogatorBusy = $state(false);
  /** Whether the Interrogator's paper pane shows an earlier text than the view's. */
  let showingOlder = $state(false);
  /** How many user-started flows (a lookup, an open, a pick, a retry...) are under way; a hand-off waits for 0. */
  let flows = $state(0);
  /** Hand-off messages (a refusal, a failed listing or setup): kept until dismissed, as `run` clears `error`. */
  let handoffNotices = $state<string[]>([]);
  /** The kind of the last failure `run` showed, so a caller can react to it. */
  let lastErrorKind: api.CommandError["kind"] | null = null;

  /** Runs a command, showing any failure; a needs_email failure opens Settings, a cancel shows nothing. */
  async function run<T>(fn: () => Promise<T>, label = OPENING): Promise<T | undefined> {
    busyText = label;
    error = null;
    lastErrorKind = null;
    try {
      return await fn();
    } catch (e) {
      const err = api.toCommandError(e);
      if (err.kind === "cancelled") return;
      api.logFailure("command", err);
      error = err.message;
      lastErrorKind = err.kind;
      if (err.kind === "needs_email") go("settings");
    } finally {
      busyText = null;
    }
  }

  /** Runs a whole flow (including its `land`) while counting it, so no hand-off opens in a gap between its runs. */
  async function flow<T>(fn: () => Promise<T>): Promise<T> {
    flows++;
    try {
      return await fn();
    } finally {
      flows--;
    }
  }

  /** Keeps a hand-off message on screen until the user dismisses it. */
  function noteHandoff(message: string) {
    handoffNotices = [...handoffNotices, message];
  }

  /** Switches screen, remembering where Settings should return to. */
  function go(s: Screen) {
    if (s === "settings" && screen !== "settings") previous = screen;
    screen = s;
  }

  /** Shows what an open/attach/reopen command produced. */
  async function land(outcome: api.OpenOutcome | undefined) {
    if (!outcome) return;
    if (outcome.status === "ready") {
      const v = await run(() => api.paperView(outcome.paper_id));
      if (v) {
        if (v.paper_id !== view?.paper_id) question = "";
        view = v;
        notices = outcome.notices;
        screen = "paper";
      } else if (outcome.notices.length) {
        // The paper landed but cannot be shown; say its notices now (a later open shows them again).
        error = [error, ...outcome.notices].filter(Boolean).join(" ");
      }
    } else {
      missing = {
        paperId: outcome.paper_id,
        reason: outcome.reason,
        unreachable: outcome.unreachable,
      };
      // Refreshed first so "Try again" is there when the screen appears.
      await refreshRecent();
      screen = "needs_full_text";
    }
  }

  /** Shows a failure the Interrogator or Settings reported (already logged); a missing model or key opens Settings. */
  function showError(err: api.CommandError) {
    error = err.message;
    if (err.kind === "needs_model" || err.kind === "needs_api_key") go("settings");
  }

  /** Reloads the recent-papers list; on failure the old list stays and the error shows. */
  async function refreshRecent() {
    recent = (await run(api.recentPapers)) ?? recent;
  }

  /** Goes back to the Papers screen with a fresh list. */
  async function toStart() {
    screen = "start";
    await refreshRecent();
  }

  /** Picks a file with the dialog and hands it to `use`. */
  function pick(use: (grant: number) => Promise<api.OpenOutcome>) {
    return flow(async () => {
      const file = await run(api.pickFile);
      if (file) await land(await run(() => use(file.id)));
    });
  }

  /** Looks a paper up again by its own stored identifier (DOI, else PMID, else PMC ID). */
  const retry = (paperId: string) => flow(() => retryNow(paperId));
  async function retryNow(paperId: string) {
    const list = await run(api.recentPapers);
    if (!list) return;
    const found = list.find((p) => p.id === paperId);
    const id = found && retryIdentifier(found.identifiers);
    if (id) await lookup(id);
    else error = "This paper is no longer in the list, or has no identifier to look it up by.";
  }

  /** Looks an identifier up; if that needs a contact email, remembers it to re-run after Settings. */
  const lookup = (id: string) =>
    flow(async () => {
      pendingRetry = null;
      const outcome = await run(() => api.openIdentifier(id), LOOKING);
      if (!outcome && lastErrorKind === "needs_email") pendingRetry = () => lookup(id);
      await land(outcome);
    });

  /** Reloads the waiting hand-offs; on failure the old list stays and the error shows. */
  async function loadHandoffs() {
    try {
      pending = await api.pendingHandoffs();
    } catch (e) {
      const err = api.toCommandError(e);
      api.logFailure("pending hand-offs", err);
      noteHandoff(`Hand-offs could not be listed: ${err.message}`);
    }
  }

  /** Opens a waiting hand-off, then reloads the list (Rust keeps the row only where a retry may help). */
  const openHandoffRow = (h: api.HandoffSummary) =>
    flow(async () => {
      tried.add(h.id);
      pendingRetry = null;
      const outcome = await run(() => api.openHandoff(h.id), handoffBusyText(h));
      if (!outcome && lastErrorKind === "needs_email") pendingRetry = () => openHandoffRow(h);
      await land(outcome);
      await loadHandoffs();
    });

  /** Removes a waiting hand-off; a failure is logged and kept on screen. */
  async function discardHandoffRow(id: number) {
    try {
      await api.discardHandoff(id);
    } catch (e) {
      const err = api.toCommandError(e);
      api.logFailure("discard hand-off", err);
      noteHandoff(`The hand-off could not be discarded: ${err.message}`);
    }
    await loadHandoffs();
  }

  /** Acts on the hand-off that is due: shows a refusal and removes it, or opens it. */
  const autoHandoff = (h: api.HandoffSummary) =>
    flow(async () => {
      // The others wait in the list, so one arrival does not open each in turn (refusals are only shown).
      pending.forEach((p) => p.refusal || tried.add(p.id));
      tried.add(h.id);
      if (h.refusal) {
        noteHandoff(h.refusal); // in place before the row goes
        await discardHandoffRow(h.id);
      } else {
        // Only the start screen shows the progress and Cancel.
        screen = "start";
        await openHandoffRow(h);
      }
    });

  // Once nothing runs, the newest hand-off not yet tried is opened (a refusal shown).
  $effect(() => {
    const h = nextHandoff(pending, tried, busy || interrogatorBusy || flows > 0 || error !== null, screen === "settings");
    if (h) untrack(() => void autoHandoff(h));
  });

  /** Asks Rust to stop the running lookup; it ends once its current request or retry wait is over. */
  async function cancelLookup() {
    busyText = CANCELLING;
    try {
      await api.stopWhilePending(api.cancelLookup, () => busyText === CANCELLING);
    } catch (e) {
      const err = api.toCommandError(e);
      api.logFailure("cancel", err);
      error = `The lookup could not be cancelled: ${err.message}`;
      if (busyText === CANCELLING) busyText = LOOKING; // offer Cancel again
    }
  }

  /** The cancel action while a lookup is running, else nothing to offer. */
  let oncancel = $derived(busyText === LOOKING ? cancelLookup : undefined);

  /** Whether the missing paper has an identifier to retry with. */
  let canRetry = $derived(
    !!missing && !!recent.find((p) => p.id === missing!.paperId && retryIdentifier(p.identifiers)),
  );

  /** Opens a dropped file, or attaches it to the paper that needs full text. */
  async function onDrop(files: api.GrantedFile[]) {
    await flow(() => dropNow(files));
  }
  /** The body of `onDrop`, run as one counted flow. */
  async function dropNow(files: api.GrantedFile[]) {
    if (files.length > 1) {
      error = "Drop one file at a time: StudyAnalyzer opens one paper at a time.";
      return;
    }
    const grant = files[0].id;
    // flows > 1: this drop is itself one flow, so another is under way.
    if (busy || flows > 1 || (screen !== "start" && screen !== "needs_full_text")) {
      error = "A file can only be opened from the Papers screen, or to give a paper its full text, and not while something else is running.";
      return;
    }
    if (screen === "start") await land(await run(() => api.openFile(grant)));
    else if (screen === "needs_full_text" && missing) {
      const id = missing.paperId;
      await land(await run(() => api.attachFile(id, grant)));
    }
  }

  /** Saves the contact email, returns to the previous screen and re-runs a lookup that waited for it. */
  const saveEmail = (value: string) =>
    flow(async () => {
      await run(() => api.setContactEmail(value));
      if (!error) {
        email = value.trim();
        screen = previous;
        const again = pendingRetry;
        pendingRetry = null;
        if (again) await again();
      }
    });

  /** Stores the debugging setting; the switch follows `keepMismatched`, so it changes only once stored. */
  async function saveKeepMismatched(keep: boolean) {
    await run(() => api.setKeepMismatched(keep));
    if (!error) keepMismatched = keep;
  }

  onMount(() => {
    // One `run`, so no load clears another's busy state or message; each load
    // that succeeds is kept, and every failure is logged and shown.
    run(async () => {
      const [r, e, k] = await Promise.allSettled([
        api.recentPapers(),
        api.getContactEmail(),
        api.getKeepMismatched(),
      ]);
      if (r.status === "fulfilled") recent = r.value;
      if (e.status === "fulfilled") email = e.value ?? "";
      if (k.status === "fulfilled") keepMismatched = k.value;
      const failed = [r, e, k].flatMap((s) =>
        s.status === "rejected" ? [api.toCommandError(s.reason)] : [],
      );
      failed.slice(1).forEach((err) => api.logFailure("startup", err));
      if (failed.length) throw { ...failed[0], message: failed.map((f) => f.message).join(" ") };
    });
    let unlisten: (() => void) | undefined;
    let unlistenHandoff: (() => void) | undefined;
    // Subscribed before the first listing: an arrival between the two is then not lost.
    api
      .onHandoff(loadHandoffs)
      .then((u) => (unlistenHandoff = u))
      .catch((e) => {
        const err = api.toCommandError(e);
        api.logFailure("hand-off setup", err);
        noteHandoff(`Hand-offs sent while the app runs will not show: ${err.message}`);
      })
      .finally(loadHandoffs);
    api
      .onFilesDropped((files) => {
        if (files.length) onDrop(files);
      })
      .then((u) => (unlisten = u))
      .catch((e) => {
        const err = api.toCommandError(e);
        api.logFailure("drag and drop setup", err);
        error = `Dropping files will not work: ${err.message}`;
      });
    return () => {
      unlisten?.();
      unlistenHandoff?.();
    };
  });
</script>

<header class="topbar">
  <div class="title">{view && screen === "paper" ? view.title : "StudyAnalyzer"}</div>
  <nav>
    <!-- Disabled while busy: another command would hide a running lookup's Cancel. -->
    {#if screen === "paper"}<button onclick={toStart} disabled={busy || interrogatorBusy}>← Papers</button>{/if}
    {#if screen !== "settings"}<button onclick={() => go("settings")} disabled={busy || interrogatorBusy}>Settings</button>{/if}
  </nav>
</header>

{#if error}<ErrorBanner message={error} ondismiss={() => (error = null)} />{/if}
{#each handoffNotices as note, i (i)}
  <ErrorBanner message={note} ondismiss={() => (handoffNotices = handoffNotices.filter((_, j) => j !== i))} />
{/each}

{#if screen === "start"}
  <StartScreen
    {recent}
    {busyText}
    {oncancel}
    handoffs={pending.filter((h) => !h.refusal)}
    onopenhandoff={openHandoffRow}
    ondiscardhandoff={(id) => discardHandoffRow(id)}
    onreopen={(id) => flow(async () => land(await run(() => api.reopenPaper(id))))}
    onpickfile={() => pick(api.openFile)}
    onpaste={(t) => flow(async () => land(await run(() => api.openPaste(t))))}
    onidentifier={lookup}
  />
{:else if screen === "needs_full_text" && missing}
  <NeedsFullText
    reason={missing.reason}
    unreachable={missing.unreachable}
    {busyText}
    {oncancel}
    {canRetry}
    onretry={() => retry(missing!.paperId)}
    onpickfile={() => pick((p) => api.attachFile(missing!.paperId, p))}
    onpaste={(t) => flow(async () => land(await run(() => api.attachPaste(missing!.paperId, t))))}
    onback={toStart}
  />
{:else if screen === "paper" && view}
  <!-- Its confirm names the latest text, so it waits while an earlier one is on screen. -->
  <FullTextBanner
    text={showingOlder ? null : bannerText(view.full_text)}
    notes={bannerNotes(view.decoded, notices)}
    onconfirm={() => flow(async () => {
      const { paper_id: id, canonical_id } = view!;
      await run(() => api.confirmFullText(id, canonical_id));
      if (!error) view = (await run(() => api.paperView(id))) ?? view;
      // A stale view is refused: reload it, keeping the message, so the next confirm names the text shown.
      else if (lastErrorKind === "invalid_input") {
        try {
          view = await api.paperView(id);
        } catch (e) {
          api.logFailure("command", api.toCommandError(e));
        }
      }
    })}
  />
  <!-- Always shown, so earlier conversations stay reachable; it takes no question about a text that neither passed the check nor was confirmed.
       Keyed on the text too: a reload that brings a newer text must not keep the old one's conversation and marks. -->
  {#key `${view.paper_id}:${view.canonical_id}`}
    <Interrogator
      {view}
      bind:question
      bind:busy={interrogatorBusy}
      bind:older={showingOlder}
      onerror={showError}
      onsettings={() => go("settings")}
    />
  {/key}
{:else if screen === "settings"}
  <Settings
    {email}
    {keepMismatched}
    onerror={showError}
    onsave={saveEmail}
    onkeepmismatched={saveKeepMismatched}
    onback={() => {
      // Left unsaved: a later, unrelated save must not re-run that old lookup.
      pendingRetry = null;
      screen = previous;
    }}
  />
{/if}

<style>
  .topbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: var(--accent);
    color: var(--paper-bg);
    padding: 0.6rem 1rem;
    font-family: var(--sans);
  }
  .topbar button {
    color: var(--paper-bg);
    border-color: var(--paper-bg);
    margin-left: 0.5rem;
  }
  .title {
    margin: 0;
    font-size: 1.1rem;
    font-weight: 600;
  }
</style>
