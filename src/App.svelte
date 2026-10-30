<script lang="ts">
  import { onMount } from "svelte";
  import { onSyncProgress } from "./lib/api";
  import { applyProgress, refreshAccounts, store } from "./lib/state.svelte";
  import AccountManager from "./components/AccountManager.svelte";
  import MailList from "./components/MailList.svelte";
  import MailDetail from "./components/MailDetail.svelte";
  import ReviewView from "./components/ReviewView.svelte";

  type View = "mails" | "accounts" | "review";
  const views: { id: View; label: string }[] = [
    { id: "mails", label: "Mails" },
    { id: "accounts", label: "Konten" },
    { id: "review", label: "Review" },
  ];

  let view = $state<View>("mails");
  let selectedId = $state<number | null>(null);

  onMount(() => {
    refreshAccounts();
    const unlisten = onSyncProgress(applyProgress);
    return () => {
      unlisten.then((f) => f());
    };
  });
</script>

<div class="shell">
  <header class="topbar">
    <span class="brand">Mail-Assistent</span>
    <nav>
      {#each views as v (v.id)}
        <button class:active={view === v.id} onclick={() => (view = v.id)}>{v.label}</button>
      {/each}
    </nav>
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
          <MailList bind:selectedId />
          <MailDetail id={selectedId} />
        </div>
      {/if}
    {:else if view === "accounts"}
      <AccountManager />
    {:else}
      <ReviewView />
    {/if}
  </main>
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

  nav button {
    border-color: transparent;
    background: transparent;
  }

  nav button.active {
    background: var(--selected);
    border-color: var(--border);
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
