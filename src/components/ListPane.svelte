<script lang="ts">
  import { store } from "../lib/state.svelte";
  import MailList from "./MailList.svelte";
  import ThreadList from "./ThreadList.svelte";
  import type { ListMode } from "../lib/types";

  let {
    mode = $bindable<ListMode>("threads"),
    selectedId = $bindable<number | null>(null),
  } = $props();

  let accountFilter = $state<number | null>(null);
  let total = $state(0);

  function changeFilter(e: Event) {
    const value = (e.currentTarget as HTMLSelectElement).value;
    accountFilter = value === "" ? null : Number(value);
    selectedId = null;
  }

  function changeMode(next: ListMode) {
    if (next === mode) return;
    mode = next;
    selectedId = null;
    total = 0;
  }

  const modes: { id: ListMode; label: string }[] = [
    { id: "threads", label: "Konversationen" },
    { id: "messages", label: "Einzeln" },
  ];
</script>

<section class="list">
  <div class="toolbar">
    <label>
      <span class="muted">Konto</span>
      <select value={accountFilter ?? ""} onchange={changeFilter}>
        <option value="">Alle Konten</option>
        {#each store.accounts as account (account.id)}
          <option value={account.id}>{account.name}</option>
        {/each}
      </select>
    </label>
    <div class="segmented" role="group" aria-label="Ansicht">
      {#each modes as m (m.id)}
        <button aria-pressed={mode === m.id} onclick={() => changeMode(m.id)}>{m.label}</button>
      {/each}
    </div>
    <span class="muted count">
      {total.toLocaleString("de-DE")}
      {mode === "threads" ? "Konversationen" : "Mails"}
    </span>
  </div>

  {#if mode === "threads"}
    <ThreadList {accountFilter} bind:selectedId bind:total />
  {:else}
    <MailList {accountFilter} bind:selectedId bind:total />
  {/if}
</section>

<style>
  .list {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--border);
    background: var(--surface);
    overflow-y: auto;
  }

  .toolbar {
    position: sticky;
    top: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
    padding: 8px 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    z-index: 1;
  }

  .toolbar label {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
  }

  .segmented button {
    border: 0;
    border-radius: 0;
    background: transparent;
    padding: 3px 10px;
  }

  .segmented button + button {
    border-left: 1px solid var(--border);
  }

  .segmented button[aria-pressed="true"] {
    background: var(--selected);
  }

  .count {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }
</style>
