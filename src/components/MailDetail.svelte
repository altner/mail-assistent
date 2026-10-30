<script lang="ts">
  import { api, errorText } from "../lib/api";
  import { longDate, sender } from "../lib/format";
  import type { Address, MessageDetail } from "../lib/types";

  let { id }: { id: number | null } = $props();

  let mail = $state<MessageDetail | null>(null);
  let error = $state<string | null>(null);

  $effect(() => {
    const current = id;
    mail = null;
    error = null;
    if (current === null) return;
    api
      .getMessage(current)
      .then((m) => {
        if (id === current) mail = m;
      })
      .catch((e) => (error = errorText(e)));
  });

  function recipients(json: string | null): string {
    if (!json) return "–";
    try {
      return (JSON.parse(json) as Address[])
        .map((a) => (a.name ? `${a.name} <${a.address}>` : a.address))
        .join(", ");
    } catch {
      return json;
    }
  }
</script>

<section class="detail">
  {#if error}
    <p class="error">{error}</p>
  {:else if !mail}
    <p class="muted placeholder">{id === null ? "Mail auswählen" : "Lade …"}</p>
  {:else}
    <header>
      <h2>{mail.subject ?? "(kein Betreff)"}</h2>
      <dl>
        <dt>Von</dt>
        <dd>
          {sender(mail.fromName, mail.fromAddr)}
          {#if mail.fromName && mail.fromAddr}<span class="muted">&lt;{mail.fromAddr}&gt;</span>{/if}
        </dd>
        <dt>An</dt>
        <dd>{recipients(mail.toAddrs)}</dd>
        <dt>Datum</dt>
        <dd>{longDate(mail.date)}</dd>
        <dt>Ordner</dt>
        <dd>{mail.folderName} · {mail.isRead ? "gelesen" : "ungelesen"}</dd>
      </dl>
    </header>
    <!-- Nur Klartext. HTML-Anzeige (abgeschotteter iframe) folgt in Phase 4. -->
    <pre class="body">{mail.bodyText ?? "(kein Textinhalt)"}</pre>
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
    margin: 0 0 12px;
    font-size: 18px;
    font-weight: 600;
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 2px 12px;
    margin: 0 0 16px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--border);
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .body {
    margin: 0;
    font: 14px/1.55 var(--font);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
