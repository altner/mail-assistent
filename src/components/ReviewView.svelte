<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { api, errorText } from "../lib/api";
  import { longDate, recipients, sender } from "../lib/format";
  import { isSuggesting, refreshAiOverview, startSuggestions, store } from "../lib/state.svelte";
  import type { Category, Decision, ReviewItem, Tag } from "../lib/types";

  const PAGE = 50;
  const BATCH = 50;

  let items = $state<ReviewItem[]>([]);
  let total = $state(0);
  let index = $state(0);
  let categories = $state<Category[]>([]);
  let tags = $state<Tag[]>([]);
  let error = $state<string | null>(null);
  let deciding = $state(false);
  /** Tags, die die aktuelle Mail bekommen soll (Vorschläge sind vorausgewählt) */
  const chosenTags = new SvelteSet<number>();

  const current = $derived(items[index] ?? null);
  const message = $derived(current?.message ?? null);

  async function load() {
    try {
      const [page, cats, tagList] = await Promise.all([
        api.listReview(PAGE, 0),
        api.listCategories(),
        api.listTags(),
      ]);
      items = page.items;
      total = page.total;
      categories = cats;
      tags = tagList;
      index = Math.min(index, Math.max(items.length - 1, 0));
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  // Neu laden, wenn Vorschläge dazukommen.
  $effect(() => {
    void store.reviewVersion;
    untrack(load);
  });

  // Vorausgewählte Tags der aktuellen Mail übernehmen.
  $effect(() => {
    const item = current;
    chosenTags.clear();
    item?.tags.forEach((t) => {
      if (t.status === "pending" || t.status === "accepted") chosenTags.add(t.id);
    });
  });

  async function decide(decision: Decision) {
    if (!current || deciding) return;
    deciding = true;
    const id = current.id;
    try {
      await api.decideReview(id, decision, [...chosenTags]);
      items = items.filter((i) => i.id !== id);
      total = Math.max(total - 1, 0);
      if (index >= items.length) index = Math.max(items.length - 1, 0);
      if (items.length === 0 && total > 0) await load();
      error = null;
      store.dataVersion++;
      refreshAiOverview();
    } catch (e) {
      error = errorText(e);
    } finally {
      deciding = false;
    }
  }

  function accept() {
    if (!current) return;
    if (current.categoryId === null) {
      error = "Kein Vorschlag vorhanden. Bitte eine Kategorie wählen oder ablehnen.";
      return;
    }
    decide({ kind: "accept" });
  }

  function choose(categoryId: number) {
    decide({ kind: "change", categoryId });
  }

  function toggleTag(id: number) {
    if (chosenTags.has(id)) chosenTags.delete(id);
    else chosenTags.add(id);
  }

  function onKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    if (target && target.closest("input, textarea, select")) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (!current) return;
    if (e.key === "Enter") accept();
    else if (e.key === "x" || e.key === "X") decide({ kind: "reject" });
    else if (e.key === "j" || e.key === "J" || e.key === "ArrowDown") index = Math.min(index + 1, items.length - 1);
    else if (e.key === "k" || e.key === "K" || e.key === "ArrowUp") index = Math.max(index - 1, 0);
    else if (/^[1-9]$/.test(e.key)) {
      const c = categories[Number(e.key) - 1];
      if (c) choose(c.id);
    } else return;
    e.preventDefault();
  }

  function percent(p: number | null): string {
    return p === null ? "–" : `${Math.round(p * 100)} %`;
  }

  onMount(() => {
    refreshAiOverview();
  });
</script>

<svelte:window onkeydown={onKeydown} />

<section class="review">
  <header class="bar">
    <div>
      <strong>Review</strong>
      <span class="muted">
        {#if store.aiOverview}
          · {store.aiOverview.pending.toLocaleString("de-DE")} offen
          · {store.aiOverview.candidates.toLocaleString("de-DE")} Mails ohne Vorschlag
        {/if}
      </span>
    </div>
    <div class="run">
      {#if store.aiProgress && isSuggesting()}
        {@const p = store.aiProgress}
        <progress max={p.total || 1} value={p.done}></progress>
        <span class="muted small">
          {p.done}/{p.total}
          {#if p.current}· {p.current}{/if}
        </span>
        <button onclick={() => api.cancelSuggestions()}>Abbrechen</button>
      {:else}
        {#if store.aiProgress}
          {@const p = store.aiProgress}
          <span class="small" class:error={p.phase === "failed"} class:muted={p.phase !== "failed"}>
            {#if p.phase === "failed"}
              {p.error ?? "Fehler"}
            {:else}
              {p.phase === "cancelled" ? "Abgebrochen" : "Fertig"}: {p.stored} Vorschläge{#if p.unreadable}, {p.unreadable} nicht auswertbar{/if}
            {/if}
          </span>
        {/if}
        <button
          class="primary"
          disabled={(store.aiOverview?.candidates ?? 1) === 0}
          onclick={() => startSuggestions(BATCH)}
        >
          Vorschläge für {BATCH} Mails holen
        </button>
      {/if}
    </div>
  </header>

  {#if error}<p class="error pad">{error}</p>{/if}

  {#if !current}
    <div class="empty muted">
      {#if store.aiOverview?.candidates === 0 && total === 0}
        Alle Mails haben einen Vorschlag und alle Vorschläge sind entschieden.
      {:else if total === 0}
        Keine offenen Vorschläge. Oben einen Lauf starten; der Server muss unter „Einstellungen“
        eingerichtet sein.
      {:else}
        Lade …
      {/if}
    </div>
  {:else if message}
    <div class="body">
      <article class="mail">
        <div class="muted small">{index + 1} von {total} · {message.folderName}</div>
        <h2>{message.subject ?? "(kein Betreff)"}</h2>
        <dl>
          <dt>Von</dt>
          <dd>{sender(message.fromName, message.fromAddr)}{#if message.fromName && message.fromAddr} <span class="muted">&lt;{message.fromAddr}&gt;</span>{/if}</dd>
          <dt>An</dt>
          <dd>{recipients(message.toAddrs)}</dd>
          <dt>Datum</dt>
          <dd>{longDate(message.date)}</dd>
        </dl>
        <pre class="text">{message.bodyText ?? "(kein Textinhalt)"}</pre>
      </article>

      <aside class="panel">
        <div class="suggestion">
          <div class="muted small">Vorschlag{#if current.model} · {current.model}{/if}</div>
          <div class="cat">
            {#if current.categoryName}
              <strong>{current.categoryName}</strong>
              <span class="muted">{percent(current.confidence)}</span>
            {:else}
              <span class="muted">keine passende Kategorie</span>
            {/if}
          </div>
          {#if current.reason}<p class="reason">{current.reason}</p>{/if}
        </div>

        <div class="actions">
          <button class="primary" disabled={deciding || current.categoryId === null} onclick={accept}>
            Übernehmen <kbd>⏎</kbd>
          </button>
          <button disabled={deciding} onclick={() => decide({ kind: "reject" })}>
            Ablehnen <kbd>X</kbd>
          </button>
        </div>

        <div class="choose">
          <div class="muted small">Andere Kategorie</div>
          <ol>
            {#each categories as c, i (c.id)}
              <li>
                <button
                  class:current={c.id === current.categoryId}
                  disabled={deciding}
                  onclick={() => choose(c.id)}
                >
                  <kbd>{i < 9 ? i + 1 : "·"}</kbd> {c.name}
                </button>
              </li>
            {/each}
          </ol>
        </div>

        {#if tags.length}
          <div class="tags">
            <div class="muted small">Tags</div>
            <div class="chips">
              {#each tags as t (t.id)}
                {@const suggested = current.tags.some((s) => s.id === t.id && s.source === "ai")}
                <button
                  class="chip"
                  class:on={chosenTags.has(t.id)}
                  title={suggested ? "von der KI vorgeschlagen" : ""}
                  onclick={() => toggleTag(t.id)}
                >
                  {t.name}{#if suggested}<span class="ai">KI</span>{/if}
                </button>
              {/each}
            </div>
          </div>
        {/if}

        <p class="muted small keys">Tasten: ⏎ übernehmen · 1–9 Kategorie · X ablehnen · J/K weiter/zurück</p>
      </aside>
    </div>
  {/if}
</section>

<style>
  .review {
    display: grid;
    grid-template-rows: auto auto 1fr;
    height: 100%;
    min-height: 0;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 16px;
    padding: 10px 20px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }

  .run {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  progress {
    width: 160px;
    accent-color: var(--accent);
  }

  .pad {
    margin: 0;
    padding: 8px 20px;
  }

  .empty {
    display: grid;
    place-content: center;
    text-align: center;
    padding: 40px;
  }

  .body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 300px;
    min-height: 0;
  }

  .mail {
    min-width: 0;
    overflow-y: auto;
    padding: 16px 24px;
  }

  h2 {
    margin: 4px 0 10px;
    font-size: 17px;
    font-weight: 600;
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 2px 12px;
    margin: 0 0 12px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .text {
    margin: 0;
    font: 14px/1.55 var(--font);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .panel {
    display: grid;
    align-content: start;
    gap: 14px;
    padding: 16px;
    border-left: 1px solid var(--border);
    background: var(--surface);
    overflow-y: auto;
  }

  .suggestion {
    display: grid;
    gap: 4px;
  }

  .cat {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-size: 16px;
  }

  .reason {
    margin: 0;
    font-size: 13px;
  }

  .actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }

  .actions button,
  .choose button {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 8px;
    width: 100%;
    text-align: left;
  }

  .choose ol {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    display: grid;
    gap: 4px;
  }

  .choose button {
    justify-content: flex-start;
    padding: 4px 8px;
  }

  .choose button.current {
    border-color: var(--accent);
  }

  kbd {
    min-width: 18px;
    padding: 0 4px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--surface-2);
    color: var(--text-muted);
    font: 11px var(--mono);
    text-align: center;
  }

  .primary kbd {
    background: transparent;
    border-color: rgb(255 255 255 / 45%);
    color: inherit;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 4px;
  }

  .chip {
    padding: 2px 10px;
    border-radius: 12px;
    font-size: 12px;
  }

  .chip.on {
    background: var(--selected);
    border-color: var(--accent);
  }

  .ai {
    margin-left: 5px;
    font-size: 10px;
    color: var(--text-muted);
  }

  .small {
    font-size: 12px;
  }

  .keys {
    margin: 0;
  }
</style>
