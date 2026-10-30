<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";
  import { api, errorText } from "../lib/api";
  import { longDate, recipients, sender, shortDate } from "../lib/format";
  import type { ThreadDetail } from "../lib/types";
  import MailBody from "./MailBody.svelte";
  import ReplyButtons from "./ReplyButtons.svelte";

  let { id }: { id: number | null } = $props();

  let thread = $state<ThreadDetail | null>(null);
  let error = $state<string | null>(null);
  /** aufgeklappte Mails; anfangs nur die neueste */
  const open = new SvelteSet<number>();

  $effect(() => {
    const current = id;
    thread = null;
    error = null;
    open.clear();
    if (current === null) return;
    api
      .getThread(current)
      .then((t) => {
        if (id !== current) return;
        thread = t;
        const newest = t.messages.at(-1);
        if (newest) open.add(newest.id);
      })
      .catch((e) => (error = errorText(e)));
  });

  function toggle(messageId: number) {
    if (open.has(messageId)) open.delete(messageId);
    else open.add(messageId);
  }

  function setAll(expand: boolean) {
    open.clear();
    if (expand) thread?.messages.forEach((m) => open.add(m.id));
  }
</script>

<section class="detail">
  {#if error}
    <p class="error">{error}</p>
  {:else if !thread}
    <p class="muted placeholder">{id === null ? "Konversation auswählen" : "Lade …"}</p>
  {:else}
    <header>
      <h2>{thread.subject ?? "(kein Betreff)"}</h2>
      <p class="muted summary">
        {thread.messageCount}
        {thread.messageCount === 1 ? "Mail" : "Mails"}
        {#if thread.unreadCount > 0}· {thread.unreadCount} ungelesen{/if}
        {#if thread.messageCount > 1}
          · <button class="link" onclick={() => setAll(true)}>alle aufklappen</button>
          · <button class="link" onclick={() => setAll(false)}>alle zuklappen</button>
        {/if}
      </p>
    </header>

    <ol>
      {#each thread.messages as m (m.id)}
        {@const expanded = open.has(m.id)}
        <li class:expanded>
          <button class="head" class:unread={!m.isRead} aria-expanded={expanded} onclick={() => toggle(m.id)}>
            <span class="dot" aria-label={m.isRead ? "gelesen" : "ungelesen"}></span>
            <span class="from">{sender(m.fromName, m.fromAddr)}</span>
            <span class="date">{expanded ? longDate(m.date) : shortDate(m.date)}</span>
            {#if !expanded}
              <span class="preview muted">{(m.bodyText ?? "").replace(/\s+/g, " ").slice(0, 160)}</span>
            {/if}
          </button>
          {#if expanded}
            <dl>
              {#if m.fromName && m.fromAddr}
                <dt>Von</dt>
                <dd>{m.fromAddr}</dd>
              {/if}
              <dt>An</dt>
              <dd>{recipients(m.toAddrs)}</dd>
              <dt>Ordner</dt>
              <dd>{m.folderName}</dd>
              {#if m.category}
                <dt>Kategorie</dt>
                <dd>{m.category}</dd>
              {/if}
            </dl>
            <div class="body">
              <div class="reply"><ReplyButtons messageId={m.id} /></div>
              <MailBody id={m.id} textFallback={m.bodyText} />
            </div>
          {/if}
        </li>
      {/each}
    </ol>
  {/if}
</section>

<style>
  .detail {
    min-height: 0;
    overflow-y: auto;
    padding: 20px 28px;
  }

  .placeholder {
    margin-top: 40px;
    text-align: center;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 18px;
    font-weight: 600;
  }

  .summary {
    margin: 0 0 16px;
  }

  .link {
    border: 0;
    padding: 0;
    background: none;
    color: var(--accent);
    cursor: pointer;
  }

  ol {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }

  li {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .head {
    display: grid;
    grid-template-columns: 10px 1fr auto;
    grid-template-areas:
      "dot from date"
      ". preview preview";
    column-gap: 8px;
    width: 100%;
    text-align: left;
    border: 0;
    background: transparent;
    padding: 8px 12px;
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
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .date {
    grid-area: date;
    color: var(--text-muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }

  .preview {
    grid-area: preview;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 2px 12px;
    margin: 0 12px;
    padding: 4px 0 10px 18px;
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

  .body {
    padding: 12px 12px 14px 30px;
  }

  .reply {
    margin-bottom: 8px;
  }
</style>
