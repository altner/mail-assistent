<script lang="ts">
  import { api, errorText } from "../lib/api";
  import { fileSize } from "../lib/format";
  import type { MailBody } from "../lib/types";

  // Anzeige einer Mail im abgeschotteten iframe (Regel 6):
  // 1. Bereinigung im Rust-Kern (kein Skript, keine Formulare, keine Handler),
  // 2. sandbox="allow-scripts" ohne allow-same-origin: das Dokument hat eine
  //    fremde Herkunft und erreicht dieses Fenster nur per postMessage; keine
  //    Formulare, keine Navigation des Fensters, keine Popups,
  // 3. eigene CSP im Dokument: nur das Brücken-Skript der App (per Hash),
  //    keine Verbindungen, Bilder nur data: (mit Freigabe https:).
  // Das Brücken-Skript (Rust, mail/html.rs) meldet Klicks, Hover und Höhe.
  // Nachrichten werden nur vom eigenen iframe angenommen und geprüft.

  let { id, textFallback }: { id: number; textFallback: string | null } = $props();

  let body = $state<MailBody | null>(null);
  let error = $state<string | null>(null);
  let allowRemote = $state(false);
  let mode = $state<"html" | "text">("html");
  let hovered = $state<string | null>(null);
  let frame = $state<HTMLIFrameElement | null>(null);
  let height = $state(200);
  let saving = $state<number | null>(null);
  let saveNote = $state<string | null>(null);

  $effect(() => {
    const current = id;
    const remote = allowRemote;
    body = null;
    error = null;
    api
      .getMessageBody(current, remote)
      .then((b) => {
        if (id === current) body = b;
      })
      .catch((e) => (error = errorText(e)));
  });

  // Bei Mailwechsel: Freigabe und Hinweise zurücksetzen.
  $effect(() => {
    void id;
    allowRemote = false;
    hovered = null;
    saveNote = null;
    mode = "html";
    height = 200;
  });

  // Link-Anzeige gehört zum iframe; in der Textansicht gibt es sie nicht.
  $effect(() => {
    if (mode === "text") hovered = null;
  });

  function onMessage(e: MessageEvent) {
    // Nur Nachrichten aus dem eigenen iframe; dessen Herkunft ist "null".
    if (!frame || e.source !== frame.contentWindow) return;
    const data = e.data as { type?: unknown; href?: unknown; value?: unknown } | null;
    if (!data || typeof data !== "object") return;
    switch (data.type) {
      case "link": {
        const href = typeof data.href === "string" ? data.href : "";
        if (/^https?:\/\//i.test(href)) api.openLink(href).catch((err) => (error = errorText(err)));
        else hovered = href;
        break;
      }
      case "hover":
        hovered = typeof data.href === "string" ? data.href : null;
        break;
      case "height":
        if (typeof data.value === "number" && Number.isFinite(data.value)) {
          // +2 für den Rahmen des iframes; Inhaltshöhe, nicht Viewport
          height = Math.min(Math.max(Math.round(data.value) + 2, 60), 20000);
        }
        break;
    }
  }

  $effect(() => {
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  });

  async function save(part: number) {
    saving = part;
    saveNote = null;
    try {
      const path = await api.saveAttachment(id, part);
      saveNote = path ? `Gespeichert: ${path}` : null;
    } catch (e) {
      saveNote = errorText(e);
    } finally {
      saving = null;
    }
  }
</script>

<div class="mailbody">
  {#if error}
    <p class="error">{error}</p>
  {:else if !body}
    <p class="muted">Lade …</p>
  {:else}
    <div class="bar">
      {#if body.isHtml}
        <div class="segmented" role="group" aria-label="Darstellung">
          <button aria-pressed={mode === "html"} onclick={() => (mode = "html")}>HTML</button>
          <button aria-pressed={mode === "text"} onclick={() => (mode = "text")}>Text</button>
        </div>
      {/if}
      {#if body.blockedImages > 0 && mode === "html"}
        <span class="muted small">
          {body.blockedImages} externe {body.blockedImages === 1 ? "Bild" : "Bilder"} blockiert
        </span>
        <button class="small" onclick={() => (allowRemote = true)}>Bilder laden</button>
      {:else if allowRemote && mode === "html"}
        <span class="muted small">Externe Bilder geladen (nur für diese Anzeige)</span>
      {/if}
    </div>

    {#if mode === "html"}
      <iframe
        bind:this={frame}
        title="Mail-Inhalt"
        sandbox="allow-scripts"
        referrerpolicy="no-referrer"
        srcdoc={body.html}
        style:height="{height}px"
      ></iframe>
    {:else}
      <pre class="text">{textFallback ?? "(kein Textinhalt)"}</pre>
    {/if}

    {#if hovered}
      <div class="status" aria-live="polite">
        {#if /^https?:\/\//i.test(hovered)}{hovered}{:else}Nicht geöffnet: {hovered}{/if}
      </div>
    {/if}

    {#if body.attachments.length}
      <ul class="attachments" aria-label="Anhänge">
        {#each body.attachments as a (a.part)}
          <li>
            <button disabled={saving !== null} onclick={() => save(a.part)} title="Speichern unter …">
              <span class="name">{a.filename}</span>
              <span class="muted small">{a.mimeType} · {fileSize(a.size)}</span>
            </button>
          </li>
        {/each}
      </ul>
      {#if saveNote}<p class="muted small note">{saveNote}</p>{/if}
    {/if}
  {/if}
</div>

<style>
  .mailbody {
    display: grid;
    gap: 8px;
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
    min-height: 24px;
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
    padding: 2px 10px;
    font-size: 12px;
  }

  .segmented button + button {
    border-left: 1px solid var(--border);
  }

  .segmented button[aria-pressed="true"] {
    background: var(--selected);
  }

  button.small {
    padding: 2px 8px;
    font-size: 12px;
  }

  .small {
    font-size: 12px;
  }

  iframe {
    width: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: #fff;
  }

  .text {
    margin: 0;
    font: 14px/1.55 var(--font);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .status {
    position: sticky;
    bottom: 0;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text-muted);
    font-size: 12px;
    overflow-wrap: anywhere;
  }

  .attachments {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .attachments button {
    display: grid;
    text-align: left;
    gap: 1px;
    padding: 6px 10px;
  }

  .name {
    overflow-wrap: anywhere;
  }

  .note {
    margin: 0;
    overflow-wrap: anywhere;
  }
</style>
