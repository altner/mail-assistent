<script lang="ts">
  import { untrack } from "svelte";
  import { api, errorText } from "../lib/api";
  import { store } from "../lib/state.svelte";
  import { sender, shortDate } from "../lib/format";
  import type { MessageSummary } from "../lib/types";

  const PAGE = 200;

  let { selectedId = $bindable<number | null>(null) } = $props();

  let accountFilter = $state<number | null>(null);
  let items = $state<MessageSummary[]>([]);
  let total = $state(0);
  let error = $state<string | null>(null);
  let loading = $state(false);

  async function load(filter: number | null, append = false) {
    loading = true;
    try {
      const offset = append ? items.length : 0;
      const limit = append ? PAGE : Math.max(PAGE, items.length);
      const page = await api.listMessages(filter, limit, offset);
      items = append ? [...items, ...page.items] : page.items;
      total = page.total;
      error = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  // Neu laden bei Filterwechsel und nach jedem Abruf-Block.
  $effect(() => {
    const filter = accountFilter;
    void store.dataVersion;
    untrack(() => load(filter));
  });

  function changeFilter(e: Event) {
    const value = (e.currentTarget as HTMLSelectElement).value;
    accountFilter = value === "" ? null : Number(value);
    items = [];
    selectedId = null;
  }

  // Pfeiltasten: Fokus und Auswahl auf die Nachbarzeile verschieben.
  function onRowKeydown(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const li = (e.currentTarget as HTMLElement).closest("li");
    const sibling = e.key === "ArrowDown" ? li?.nextElementSibling : li?.previousElementSibling;
    const button = sibling?.querySelector("button");
    button?.focus();
    button?.click();
  }

  const accountNames = $derived(new Map(store.accounts.map((a) => [a.id, a.name])));
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
    <span class="muted count">{total.toLocaleString("de-DE")} Mails</span>
  </div>

  {#if error}
    <p class="error pad">{error}</p>
  {:else if items.length === 0 && !loading}
    <p class="muted pad">Keine Mails. Unter „Konten“ einen Abruf starten.</p>
  {/if}

  <ul aria-label="Mail-Liste">
    {#each items as m (m.id)}
      <li>
        <button
          class="row"
          class:unread={!m.isRead}
          class:selected={m.id === selectedId}
          onclick={() => (selectedId = m.id)}
          onkeydown={onRowKeydown}
        >
          <span class="dot" title={m.isRead ? "gelesen" : "ungelesen"} aria-label={m.isRead ? "gelesen" : "ungelesen"}></span>
          <span class="from">{sender(m.fromName, m.fromAddr)}</span>
          <span class="date">{shortDate(m.date)}</span>
          <span class="subject">{m.subject ?? "(kein Betreff)"}</span>
          <span class="meta muted">
            {#if accountFilter === null}{accountNames.get(m.accountId)} · {/if}{m.folderName}
          </span>
        </button>
      </li>
    {/each}
  </ul>

  {#if items.length < total}
    <div class="pad">
      <button disabled={loading} onclick={() => load(accountFilter, true)}>
        Weitere laden ({items.length} von {total})
      </button>
    </div>
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
    align-items: center;
    justify-content: space-between;
    gap: 8px;
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

  .count {
    font-variant-numeric: tabular-nums;
  }

  .pad {
    padding: 12px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .row {
    display: grid;
    grid-template-columns: 10px 1fr auto;
    grid-template-areas:
      "dot from date"
      ". subject subject"
      ". meta meta";
    column-gap: 8px;
    width: 100%;
    text-align: left;
    border: 0;
    border-bottom: 1px solid var(--border);
    border-radius: 0;
    background: transparent;
    padding: 8px 12px;
  }

  .row.selected {
    background: var(--selected);
  }

  .dot {
    grid-area: dot;
    align-self: center;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .unread .dot {
    background: var(--accent);
  }

  .from {
    grid-area: from;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .unread .from,
  .unread .subject {
    font-weight: 600;
  }

  .date {
    grid-area: date;
    color: var(--text-muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .subject {
    grid-area: subject;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    grid-area: meta;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
