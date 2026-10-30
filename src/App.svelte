<script lang="ts">
  import { onMount } from "svelte";
  import { onAiProgress, onSendStatus, onSyncProgress } from "./lib/api";
  import {
    applyAiProgress,
    applyProgress,
    applySendStatus,
    isSyncing,
    refreshAccounts,
    refreshAiOverview,
    refreshDraftCount,
    store,
    syncAll,
  } from "./lib/state.svelte";
  import type { ListMode } from "./lib/types";
  import AccountManager from "./components/AccountManager.svelte";
  import ListPane from "./components/ListPane.svelte";
  import MailDetail from "./components/MailDetail.svelte";
  import ThreadView from "./components/ThreadView.svelte";
  import ReviewView from "./components/ReviewView.svelte";
  import SettingsView from "./components/SettingsView.svelte";
  import DraftsView from "./components/DraftsView.svelte";
  import ContactsView from "./components/ContactsView.svelte";
  import Composer from "./components/Composer.svelte";

  type View = "mails" | "drafts" | "contacts" | "accounts" | "review" | "settings";
  const views: { id: View; label: string }[] = [
    { id: "mails", label: "Mails" },
    { id: "drafts", label: "Entwürfe" },
    { id: "review", label: "Review" },
    { id: "contacts", label: "Kontakte" },
    { id: "accounts", label: "Konten" },
    { id: "settings", label: "Einstellungen" },
  ];

  let view = $state<View>("mails");
  let selectedId = $state<number | null>(null);

  // Gewählte Listenansicht nur als Komfort im Browser-Speicher merken.
  const MODE_KEY = "list-mode";
  function storedMode(): ListMode {
    try {
      return localStorage.getItem(MODE_KEY) === "messages" ? "messages" : "threads";
    } catch {
      return "threads";
    }
  }
  let listMode = $state<ListMode>(storedMode());
  $effect(() => {
    try {
      localStorage.setItem(MODE_KEY, listMode);
    } catch {
      // ohne Speicher gilt beim nächsten Start wieder die Vorgabe
    }
  });

  const syncingCount = $derived(store.accounts.filter((a) => isSyncing(a.id)).length);

  onMount(() => {
    // Einmal beim Start alle Konten abrufen (nur lesend, Regel 1).
    refreshAccounts().then(syncAll);
    refreshAiOverview();
    refreshDraftCount();
    const unlisten = Promise.all([
      onSyncProgress(applyProgress),
      onAiProgress(applyAiProgress),
      onSendStatus(applySendStatus),
    ]);
    return () => {
      unlisten.then((fns) => fns.forEach((f) => f()));
    };
  });
</script>

<div class="shell">
  <header class="topbar">
    <span class="brand">Mail-Assistent</span>
    <nav>
      {#each views as v (v.id)}
        <button class:active={view === v.id} onclick={() => (view = v.id)}>
          {v.label}
          {#if v.id === "review" && store.aiOverview?.pending}
            <span class="badge">{store.aiOverview.pending.toLocaleString("de-DE")}</span>
          {/if}
          {#if v.id === "drafts" && store.draftCount}
            <span class="badge soft">{store.draftCount}</span>
          {/if}
        </button>
      {/each}
    </nav>
    {#if store.accounts.length}
      <button
        class="sync"
        disabled={syncingCount > 0}
        onclick={syncAll}
        title="Alle Konten gleichzeitig abrufen (nur lesend)"
      >
        {syncingCount > 0 ? `Ruft ab (${syncingCount}/${store.accounts.length}) …` : "Alle abrufen"}
      </button>
    {/if}
  </header>

  <main>
    {#if view === "mails"}
      {#if store.accounts.length === 0}
        <div class="empty">
          <p>Noch kein Konto eingerichtet.</p>
          <button class="primary" onclick={() => (view = "accounts")}>Konto anlegen</button>
        </div>
      {:else}
        <div class="split">
          <ListPane bind:mode={listMode} bind:selectedId />
          {#if listMode === "threads"}
            <ThreadView id={selectedId} />
          {:else}
            <MailDetail id={selectedId} />
          {/if}
        </div>
      {/if}
    {:else if view === "accounts"}
      <AccountManager />
    {:else if view === "review"}
      <ReviewView />
    {:else if view === "drafts"}
      <DraftsView />
    {:else if view === "contacts"}
      <ContactsView />
    {:else}
      <SettingsView />
    {/if}
  </main>

  {#if store.composerDraftId !== null}
    <Composer id={store.composerDraftId} />
  {/if}
</div>

<style>
  .shell {
    display: grid;
    grid-template-rows: auto 1fr;
    height: 100%;
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: 24px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .brand {
    font-weight: 600;
  }

  nav {
    display: flex;
    gap: 4px;
  }

  .sync {
    margin-left: auto;
  }

  nav button {
    border-color: transparent;
    background: transparent;
  }

  nav button.active {
    background: var(--selected);
    border-color: var(--border);
  }

  .badge.soft {
    background: var(--surface-2);
    color: var(--text-muted);
  }

  .badge {
    display: inline-block;
    min-width: 18px;
    margin-left: 4px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--accent);
    color: var(--accent-text);
    font-size: 11px;
    font-weight: 600;
    line-height: 17px;
    text-align: center;
  }

  main {
    min-height: 0;
    overflow: hidden;
  }

  .split {
    display: grid;
    grid-template-columns: minmax(360px, 42%) 1fr;
    height: 100%;
  }

  .empty {
    display: grid;
    place-content: center;
    justify-items: center;
    height: 100%;
    gap: 8px;
  }
</style>
