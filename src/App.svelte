<script lang="ts">
  import { onMount } from "svelte";
  import ErrorBanner from "./components/ErrorBanner.svelte";
  import FullTextBanner from "./components/FullTextBanner.svelte";
  import NeedsFullText from "./components/NeedsFullText.svelte";
  import PaperPane from "./components/PaperPane.svelte";
  import Settings from "./components/Settings.svelte";
  import StartScreen from "./components/StartScreen.svelte";
  import * as api from "./lib/api";
  import { bannerText, encodingNote } from "./lib/paper";

  type Screen = "start" | "needs_full_text" | "paper" | "settings";
  let screen = $state<Screen>("start");
  let previous: Screen = "start";
  let error = $state<string | null>(null);
  let busy = $state(false);
  let recent = $state<api.PaperSummary[]>([]);
  let view = $state<api.PaperView | null>(null);
  let missing = $state<{ paperId: string; reason: string; unreachable: boolean } | null>(null);
  let lastIdentifier = "";
  let email = $state("");

  /** Runs a command, showing any failure; a needs_email failure opens Settings. */
  async function run<T>(fn: () => Promise<T>): Promise<T | undefined> {
    busy = true;
    error = null;
    try {
      return await fn();
    } catch (e) {
      const err = api.toCommandError(e);
      error = err.message;
      if (err.kind === "needs_email") go("settings");
    } finally {
      busy = false;
    }
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
        view = v;
        screen = "paper";
      }
    } else {
      missing = {
        paperId: outcome.paper_id,
        reason: outcome.reason,
        unreachable: outcome.unreachable,
      };
      screen = "needs_full_text";
    }
  }

  async function refreshRecent() {
    recent = (await run(api.recentPapers)) ?? recent;
  }

  async function toStart() {
    screen = "start";
    await refreshRecent();
  }

  /** Picks a file with the dialog and hands it to `use`. */
  async function pick(use: (path: string) => Promise<api.OpenOutcome>) {
    const path = await run(api.pickFile);
    if (path) await land(await run(() => use(path)));
  }

  async function onDrop(path: string) {
    if (busy) return;
    if (screen === "start") await land(await run(() => api.openFile(path)));
    else if (screen === "needs_full_text" && missing) {
      const id = missing.paperId;
      await land(await run(() => api.attachFile(id, path)));
    }
  }

  async function saveEmail(value: string) {
    await run(() => api.setContactEmail(value));
    if (!error) {
      email = value.trim();
      screen = previous;
    }
  }

  onMount(() => {
    refreshRecent();
    run(api.getContactEmail).then((e) => (email = e ?? ""));
    let unlisten: (() => void) | undefined;
    import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((ev) => {
          if (ev.payload.type === "drop" && ev.payload.paths.length) onDrop(ev.payload.paths[0]);
        }),
      )
      .then((u) => (unlisten = u))
      .catch((e) => (error = api.toCommandError(e).message));
    return () => unlisten?.();
  });
</script>

<header class="topbar">
  <h1>{view && screen === "paper" ? view.title : "StudyAnalyzer"}</h1>
  <nav>
    {#if screen === "paper"}<button onclick={toStart}>← Papers</button>{/if}
    {#if screen !== "settings"}<button onclick={() => go("settings")}>Settings</button>{/if}
  </nav>
</header>

{#if error}<ErrorBanner message={error} ondismiss={() => (error = null)} />{/if}

{#if screen === "start"}
  <StartScreen
    {recent}
    {busy}
    onreopen={async (id) => land(await run(() => api.reopenPaper(id)))}
    onpickfile={() => pick(api.openFile)}
    onpaste={async (t) => land(await run(() => api.openPaste(t)))}
    onidentifier={async (i) => {
      lastIdentifier = i;
      await land(await run(() => api.openIdentifier(i)));
    }}
  />
{:else if screen === "needs_full_text" && missing}
  <NeedsFullText
    reason={missing.reason}
    unreachable={missing.unreachable}
    {busy}
    onretry={async () => land(await run(() => api.openIdentifier(lastIdentifier)))}
    onpickfile={() => pick((p) => api.attachFile(missing!.paperId, p))}
    onpaste={async (t) => land(await run(() => api.attachPaste(missing!.paperId, t)))}
    onback={toStart}
  />
{:else if screen === "paper" && view}
  <FullTextBanner
    text={bannerText(view.full_text)}
    note={encodingNote(view.decoded)}
    onconfirm={async () => {
      const id = view!.paper_id;
      await run(() => api.confirmFullText(id));
      if (!error) view = await run(() => api.paperView(id)) ?? view;
    }}
  />
  <PaperPane {view} />
{:else if screen === "settings"}
  <Settings {email} onsave={saveEmail} onback={() => (screen = previous)} />
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
  h1 {
    margin: 0;
    font-size: 1.1rem;
    font-weight: 600;
  }
</style>
