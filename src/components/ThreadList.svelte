<script lang="ts">
  import { untrack } from "svelte";
  import { api, errorText } from "../lib/api";
  import { store } from "../lib/state.svelte";
  import { shortDate } from "../lib/format";
  import { onRowKeydown } from "../lib/listnav";
  import type { ThreadSummary } from "../lib/types";

  const PAGE = 200;

  let {
    accountFilter,
    selectedId = $bindable<number | null>(null),
    total = $bindable(0),
  }: { accountFilter: number | null; selectedId?: number | null; total?: number } = $props();

  let items = $state<ThreadSummary[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(false);

  async function load(filter: number | null, append = false) {
    loading = true;
    try {
      const offset = append ? items.length : 0;
      // Beim Neuladen nach einem Abruf so viele Zeilen behalten wie sichtbar.
      const limit = append ? PAGE : Math.max(PAGE, items.length);
      const page = await api.listThreads(filter, limit, offset);
      items = append ? [...items, ...page.items] : page.items;
      total = page.total;
      error = null;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  // Bei Filterwechsel von vorn, nach jedem Abruf-Block neu laden.
  let lastFilter: number | null | undefined;
  $effect(() => {
    const filter = accountFilter;
    void store.dataVersion;
    untrack(() => {
      if (filter !== lastFilter) items = [];
      lastFilter = filter;
      load(filter);
    });
  });

  const accountNames = $derived(new Map(store.accounts.map((a) => [a.id, a.name])));
</script>

{#if error}
  <p class="error pad">{error}</p>
{:else if items.length === 0 && !loading}
  <p class="muted pad">Keine Mails. Unter „Konten“ einen Abruf starten.</p>
{/if}

<ul aria-label="Konversationen">
  {#each items as t (t.id)}
    {@const unread = t.unreadCount > 0}
    <li>
      <button
        class="row"
        class:unread
        class:selected={t.id === selectedId}
        onclick={() => (selectedId = t.id)}
        onkeydown={onRowKeydown}
      >
        <span
          class="dot"
          title={unread ? `${t.unreadCount} ungelesen` : "gelesen"}
          aria-label={unread ? `${t.unreadCount} ungelesen` : "gelesen"}
        ></span>
        <span class="from">
          {t.participants.join(", ") || "(unbekannt)"}
        </span>
        <span class="date">{shortDate(t.lastDate)}</span>
        <span class="subject">
          {#if t.messageCount > 1}<span class="badge" title="{t.messageCount} Mails">{t.messageCount}</span>{/if}
          {t.subject ?? "(kein Betreff)"}
        </span>
        {#if accountFilter === null}
          <span class="meta muted">{accountNames.get(t.accountId)}</span>
        {/if}
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

<style>
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

  .badge {
    display: inline-block;
    min-width: 18px;
    margin-right: 4px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--surface-2);
    color: var(--text-muted);
    font-size: 11px;
    font-weight: 600;
    line-height: 17px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }

  .meta {
    grid-area: meta;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
