<script lang="ts">
  import { untrack } from "svelte";
  import { api, errorText } from "../lib/api";
  import { shortDate } from "../lib/format";
  import { composeNew, openDraft, refreshDraftCount, store } from "../lib/state.svelte";
  import type { Draft } from "../lib/types";

  let drafts = $state<Draft[]>([]);
  let error = $state<string | null>(null);

  async function load() {
    try {
      drafts = await api.listDrafts();
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  $effect(() => {
    void store.draftsVersion;
    untrack(load);
  });

  const accountNames = $derived(new Map(store.accounts.map((a) => [a.id, a.name])));

  async function remove(d: Draft) {
    if (!confirm(`Entwurf „${d.subject || "(kein Betreff)"}“ verwerfen?`)) return;
    try {
      await api.deleteDraft(d.id);
      store.draftsVersion++;
      refreshDraftCount();
    } catch (e) {
      error = errorText(e);
    }
  }

  function kindLabel(k: Draft["kind"]): string {
    return k === "forward" ? "Weiterleitung" : k === "new" ? "Neue Mail" : "Antwort";
  }

  function preview(html: string): string {
    const div = document.createElement("div");
    div.innerHTML = html;
    div.querySelectorAll("blockquote, .signature, .quote-header, .forward-header").forEach((n) => n.remove());
    return (div.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 120);
  }
</script>

<section class="drafts">
  <header>
    <h2>Entwürfe</h2>
    {#if store.accounts.length}
      <div class="new">
        <span class="muted small">Neue Mail über</span>
        {#each store.accounts as a (a.id)}
          <button onclick={() => composeNew(a.id)}>{a.name}</button>
        {/each}
      </div>
    {/if}
  </header>
  {#if error}<p class="error">{error}</p>{/if}
  {#if drafts.length === 0}
    <p class="muted">Keine Entwürfe. Antworten oder Weiterleiten legt einen an, oben eine neue Mail.</p>
  {/if}
  <ul>
    {#each drafts as d (d.id)}
      {@const status = store.sendStatus[d.id]}
      <li>
        <button class="row" onclick={() => openDraft(d.id)}>
          <span class="kind muted small">{kindLabel(d.kind)}</span>
          <span class="date muted small">{shortDate(d.updatedAt)}</span>
          <span class="to">{d.to.map((r) => r.name ?? r.address).join(", ") || "(kein Empfänger)"}</span>
          <span class="subject">{d.subject || "(kein Betreff)"}</span>
          <span class="meta muted small">
            {accountNames.get(d.accountId)}{#if d.attachments.length} · {d.attachments.length} Anhänge{/if}
            {#if status?.phase === "waiting"} · wird in {status.secondsLeft} s gesendet{/if}
            {#if status?.phase === "failed"} · <span class="error">Senden fehlgeschlagen</span>{/if}
          </span>
          <span class="preview muted small">{preview(d.bodyHtml)}</span>
        </button>
        <button class="x" aria-label="Entwurf verwerfen" onclick={() => remove(d)}>×</button>
      </li>
    {/each}
  </ul>
</section>

<style>
  .drafts {
    height: 100%;
    overflow-y: auto;
    padding: 20px 28px;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 12px;
  }

  h2 {
    margin: 0;
    font-size: 18px;
  }

  .new {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }

  li {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: start;
    gap: 4px;
  }

  .row {
    display: grid;
    grid-template-columns: 1fr auto;
    grid-template-areas:
      "kind date"
      "to to"
      "subject subject"
      "meta meta"
      "preview preview";
    gap: 2px 8px;
    width: 100%;
    text-align: left;
    padding: 10px 14px;
  }

  .kind { grid-area: kind; }
  .date { grid-area: date; }
  .to { grid-area: to; }
  .subject { grid-area: subject; font-weight: 600; }
  .meta { grid-area: meta; }
  .preview { grid-area: preview; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  .x {
    border: 0;
    background: transparent;
    color: var(--text-muted);
    font-size: 16px;
    padding: 8px;
  }

  .small {
    font-size: 12px;
  }
</style>
