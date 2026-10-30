<script lang="ts">
  import { untrack } from "svelte";
  import { api, errorText } from "../lib/api";
  import { fileSize } from "../lib/format";
  import { closeComposer, refreshDraftCount, store } from "../lib/state.svelte";
  import type { Draft, Identity, Recipient } from "../lib/types";
  import RecipientField from "./RecipientField.svelte";

  // Editor für einen Entwurf. Gesendet wird nur über den Bestätigungsdialog
  // unten; der Rust-Befehl send_draft hat keinen anderen Aufrufer.

  let { id }: { id: number } = $props();

  let draft = $state<Draft | null>(null);
  let identities = $state<Identity[]>([]);
  let error = $state<string | null>(null);
  let to = $state<Recipient[]>([]);
  let cc = $state<Recipient[]>([]);
  let bcc = $state<Recipient[]>([]);
  let showCcBcc = $state(false);
  let subject = $state("");
  let identityId = $state<number | null>(null);
  let editor = $state<HTMLDivElement | null>(null);
  let dirty = $state(false);
  let saving = $state(false);
  let savedAt = $state<string | null>(null);
  let confirming = $state(false);
  let aiHint = $state("");
  let aiBusy = $state(false);
  let linkUrl = $state<string | null>(null);
  let saveTimer: ReturnType<typeof setTimeout> | undefined;

  const accountName = $derived(store.accounts.find((a) => a.id === draft?.accountId)?.name ?? "");
  const identity = $derived(identities.find((i) => i.id === identityId) ?? null);
  const status = $derived(store.sendStatus[id] ?? null);
  const busy = $derived(status?.phase === "waiting" || status?.phase === "sending");

  $effect(() => {
    const current = id;
    untrack(() => load(current));
  });

  async function load(current: number) {
    try {
      const d = await api.getDraft(current);
      if (id !== current) return;
      draft = d;
      to = d.to;
      cc = d.cc;
      bcc = d.bcc;
      showCcBcc = d.cc.length > 0 || d.bcc.length > 0;
      subject = d.subject;
      identityId = d.identityId;
      identities = await api.listIdentities(d.accountId);
      if (identityId === null && identities.length) identityId = identities[0].id;
      // Inhalt setzen, sobald der Editor im DOM ist.
      queueMicrotask(() => {
        if (editor) editor.innerHTML = d.bodyHtml;
      });
      dirty = false;
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  function markDirty() {
    dirty = true;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, 800);
  }

  async function save() {
    if (!draft || !editor) return;
    clearTimeout(saveTimer);
    saving = true;
    try {
      await api.updateDraft(id, {
        identityId,
        to,
        cc,
        bcc,
        subject,
        bodyHtml: editor.innerHTML,
      });
      dirty = false;
      savedAt = new Date().toLocaleTimeString("de-DE", { hour: "2-digit", minute: "2-digit" });
      store.draftsVersion++;
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }

  /** Beim Identitätswechsel die Signatur im Text austauschen. */
  function changeIdentity(e: Event) {
    const next = Number((e.currentTarget as HTMLSelectElement).value);
    const old = identities.find((i) => i.id === identityId);
    const neu = identities.find((i) => i.id === next);
    identityId = next;
    if (editor && neu) {
      const sig = editor.querySelector(".signature");
      const html = signatureHtml(neu);
      if (sig) {
        if (html) sig.outerHTML = html;
        else sig.remove();
      } else if (html && !old?.signatureHtml && !old?.signatureText) {
        // vorher keine Signatur: hinter der ersten Zeile einsetzen
        editor.firstElementChild?.insertAdjacentHTML("afterend", html);
      }
    }
    markDirty();
  }

  function signatureHtml(i: Identity): string {
    if (i.signatureHtml) return `<div class="signature">-- <br>${i.signatureHtml}</div>`;
    if (i.signatureText) {
      const esc = i.signatureText
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/\n/g, "<br>");
      return `<div class="signature">-- <br>${esc}</div>`;
    }
    return "";
  }

  // --- Formatierung (execCommand ist veraltet, aber in WebKit zuverlässig) ---
  function cmd(name: string, value?: string) {
    editor?.focus();
    document.execCommand(name, false, value);
    markDirty();
  }

  function insertLink() {
    const url = linkUrl?.trim();
    linkUrl = null;
    if (!url) return;
    if (!/^https?:\/\//i.test(url)) {
      error = "Links müssen mit http:// oder https:// beginnen.";
      return;
    }
    cmd("createLink", url);
  }

  function onPaste(e: ClipboardEvent) {
    // Nur Text einfügen, keine fremden Formatierungen oder Skripte.
    e.preventDefault();
    const text = e.clipboardData?.getData("text/plain") ?? "";
    document.execCommand("insertText", false, text);
    markDirty();
  }

  // --- Anhänge ---
  async function addFile() {
    try {
      const d = await api.addDraftFile(id);
      if (d) draft = d;
    } catch (e) {
      error = errorText(e);
    }
  }

  async function removeAttachment(i: number) {
    try {
      draft = await api.removeDraftAttachment(id, i);
    } catch (e) {
      error = errorText(e);
    }
  }

  // --- KI-Entwurf ---
  async function aiDraft() {
    if (!editor) return;
    aiBusy = true;
    error = null;
    try {
      await save();
      const html = await api.aiDraft(id, aiHint);
      // Vor Signatur und Zitat einsetzen; der leere Absatz oben wird ersetzt.
      const first = editor.firstElementChild;
      if (first && first.textContent?.trim() === "" && !first.classList.contains("signature")) {
        first.outerHTML = html + "<div><br></div>";
      } else {
        editor.insertAdjacentHTML("afterbegin", html + "<div><br></div>");
      }
      markDirty();
    } catch (e) {
      error = errorText(e);
    } finally {
      aiBusy = false;
    }
  }

  // --- Senden ---
  async function openConfirm() {
    await save();
    if (!to.length && !cc.length && !bcc.length) {
      error = "Mindestens ein Empfänger ist nötig.";
      return;
    }
    if (!draft?.accountId || !store.accounts.find((a) => a.id === draft?.accountId)?.smtpHost) {
      error = "Für dieses Konto ist kein SMTP-Server eingetragen (Reiter „Konten“).";
      return;
    }
    error = null;
    confirming = true;
  }

  async function confirmSend() {
    confirming = false;
    try {
      const result = await api.sendDraft(id);
      if (result.phase === "sent") {
        closeComposer();
        refreshDraftCount();
      }
    } catch (e) {
      error = errorText(e);
    }
  }

  async function undo() {
    try {
      await api.cancelSend(id);
    } catch (e) {
      error = errorText(e);
    }
  }

  async function discard() {
    if (!confirm("Entwurf verwerfen?")) return;
    try {
      await api.deleteDraft(id);
      store.draftsVersion++;
      refreshDraftCount();
      closeComposer();
    } catch (e) {
      error = errorText(e);
    }
  }

  function close() {
    clearTimeout(saveTimer);
    if (dirty) save();
    closeComposer();
  }

  function fmt(list: Recipient[]): string {
    return list.map((r) => (r.name ? `${r.name} <${r.address}>` : r.address)).join(", ");
  }
</script>

<aside class="composer" aria-label="Entwurf">
  <header>
    <strong>
      {#if draft?.kind === "forward"}Weiterleiten{:else if draft?.kind === "new"}Neue Mail{:else}Antwort{/if}
    </strong>
    <span class="muted small">
      {#if saving}Speichere …{:else if dirty}ungespeichert{:else if savedAt}gespeichert {savedAt}{/if}
    </span>
    <button class="close" aria-label="Schließen" onclick={close}>×</button>
  </header>

  {#if error}<p class="error pad">{error}</p>{/if}

  {#if draft}
    <div class="fields">
      <div class="row">
        <span class="label muted">Von</span>
        <select value={identityId ?? ""} onchange={changeIdentity} disabled={busy}>
          {#each identities as i (i.id)}
            <option value={i.id}>{i.name} &lt;{i.email}&gt;</option>
          {/each}
        </select>
        <span class="muted small">{accountName}</span>
      </div>
      <RecipientField label="An" bind:recipients={to} onchange={markDirty} />
      {#if showCcBcc}
        <RecipientField label="Cc" bind:recipients={cc} onchange={markDirty} />
        <RecipientField label="Bcc" bind:recipients={bcc} onchange={markDirty} />
      {:else}
        <button class="link" onclick={() => (showCcBcc = true)}>Cc/Bcc hinzufügen</button>
      {/if}
      <div class="row">
        <span class="label muted">Betreff</span>
        <input bind:value={subject} oninput={markDirty} placeholder="Betreff" />
      </div>
    </div>

    <div class="toolbar" role="toolbar" aria-label="Formatierung">
      <button title="Fett (⌘B)" onclick={() => cmd("bold")}><b>F</b></button>
      <button title="Kursiv (⌘I)" onclick={() => cmd("italic")}><i>K</i></button>
      <button title="Aufzählung" onclick={() => cmd("insertUnorderedList")}>• Liste</button>
      <button title="Nummerierung" onclick={() => cmd("insertOrderedList")}>1. Liste</button>
      <button title="Zitat" onclick={() => cmd("formatBlock", "blockquote")}>„ Zitat</button>
      <button title="Link" onclick={() => (linkUrl = "https://")}>Link</button>
      <button title="Formatierung entfernen" onclick={() => cmd("removeFormat")}>Tx</button>
      <span class="spacer"></span>
      <button onclick={addFile} disabled={busy}>Datei anhängen</button>
    </div>
    {#if linkUrl !== null}
      <form class="linkform" onsubmit={(e) => { e.preventDefault(); insertLink(); }}>
        <input bind:value={linkUrl} placeholder="https://…" aria-label="Link-Adresse" />
        <button type="submit" class="primary">Einfügen</button>
        <button type="button" onclick={() => (linkUrl = null)}>Abbrechen</button>
      </form>
    {/if}

    <div
      bind:this={editor}
      class="editor"
      contenteditable={!busy}
      role="textbox"
      aria-multiline="true"
      aria-label="Nachricht"
      tabindex="0"
      oninput={markDirty}
      onpaste={onPaste}
    ></div>

    {#if draft.attachments.length}
      <ul class="attachments" aria-label="Anhänge">
        {#each draft.attachments as a, i (a.source + a.name + i)}
          <li>
            <span>{a.name}</span>
            <span class="muted small">{fileSize(a.size)}</span>
            <button class="x" aria-label="{a.name} entfernen" disabled={busy} onclick={() => removeAttachment(i)}>×</button>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="ai">
      <input bind:value={aiHint} placeholder="Hinweis für die KI, z. B. „kurz zusagen“" aria-label="Hinweis für die KI" disabled={aiBusy || busy} />
      <button onclick={aiDraft} disabled={aiBusy || busy}>{aiBusy ? "Schreibt …" : "KI-Entwurf"}</button>
    </div>

    <footer>
      {#if status?.phase === "waiting"}
        <div class="undo">
          <span>Wird in {status.secondsLeft} s gesendet</span>
          <button class="primary" onclick={undo}>Rückgängig</button>
        </div>
      {:else if status?.phase === "sending"}
        <span class="muted">Wird gesendet …</span>
      {:else}
        <button class="primary" onclick={openConfirm} disabled={busy}>Senden …</button>
        <button onclick={discard} disabled={busy}>Verwerfen</button>
        {#if status?.phase === "cancelled"}<span class="muted small">Senden abgebrochen, Entwurf bleibt.</span>{/if}
        {#if status?.phase === "failed"}<span class="error small">{status.error}</span>{/if}
      {/if}
    </footer>
  {:else if !error}
    <p class="muted pad">Lade …</p>
  {/if}

  {#if confirming && draft}
    <div class="overlay" role="dialog" aria-modal="true" aria-labelledby="confirm-title">
      <div class="dialog">
        <h3 id="confirm-title">Jetzt senden?</h3>
        <dl>
          <dt>Konto</dt><dd>{accountName}</dd>
          <dt>Von</dt><dd>{identity ? `${identity.name} <${identity.email}>` : "–"}</dd>
          <dt>An</dt><dd>{fmt(to) || "–"}</dd>
          {#if cc.length}<dt>Cc</dt><dd>{fmt(cc)}</dd>{/if}
          {#if bcc.length}<dt>Bcc</dt><dd>{fmt(bcc)}</dd>{/if}
          <dt>Betreff</dt><dd>{subject || "(kein Betreff)"}</dd>
          {#if draft.attachments.length}
            <dt>Anhänge</dt><dd>{draft.attachments.map((a) => a.name).join(", ")}</dd>
          {/if}
        </dl>
        <p class="muted small">
          Die Mail geht über den SMTP-Server des Kontos raus und wird lokal unter „Gesendet (lokal)“ abgelegt.
          Am Server-Postfach ändert sich nichts.
        </p>
        <div class="actions">
          <button class="primary" onclick={confirmSend}>Jetzt senden</button>
          <button onclick={() => (confirming = false)}>Abbrechen</button>
        </div>
      </div>
    </div>
  {/if}
</aside>

<style>
  .composer {
    position: fixed;
    top: 48px;
    right: 0;
    bottom: 0;
    width: min(680px, 100%);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-left: 1px solid var(--border);
    box-shadow: -8px 0 24px rgb(0 0 0 / 10%);
    z-index: 20;
  }

  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px 8px 16px;
    border-bottom: 1px solid var(--border);
  }

  header .small {
    margin-left: auto;
  }

  .close {
    border: 0;
    background: transparent;
    font-size: 18px;
    line-height: 1;
    padding: 2px 6px;
  }

  .pad {
    margin: 0;
    padding: 6px 16px;
  }

  .fields {
    padding: 4px 16px;
  }

  .row {
    display: grid;
    grid-template-columns: 56px 1fr auto;
    align-items: center;
    gap: 6px;
    padding: 4px 0;
    border-bottom: 1px solid var(--border);
  }

  .row input,
  .row select {
    border: 0;
    background: transparent;
    padding: 3px 4px;
  }

  .label {
    font-size: 13px;
  }

  .link {
    border: 0;
    background: none;
    color: var(--accent);
    padding: 4px 0;
    font-size: 12px;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--border);
  }

  .toolbar button {
    padding: 2px 8px;
    font-size: 12px;
  }

  .spacer {
    flex: 1;
  }

  .linkform {
    display: flex;
    gap: 6px;
    padding: 6px 16px;
    border-bottom: 1px solid var(--border);
  }

  .linkform input {
    flex: 1;
  }

  .editor {
    flex: 1;
    min-height: 120px;
    overflow-y: auto;
    padding: 12px 16px;
    font: 14px/1.5 var(--font);
    outline: none;
  }

  .editor :global(blockquote) {
    margin: 4px 0 4px 8px;
    padding-left: 12px;
    border-left: 2px solid var(--border);
    color: var(--text-muted);
  }

  .editor :global(.signature) {
    color: var(--text-muted);
  }

  .editor :global(img) {
    max-width: 100%;
  }

  .attachments {
    list-style: none;
    margin: 0;
    padding: 6px 16px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    border-top: 1px solid var(--border);
  }

  .attachments li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    font-size: 13px;
  }

  .x {
    border: 0;
    background: transparent;
    color: var(--text-muted);
    padding: 0 4px;
  }

  .ai {
    display: flex;
    gap: 6px;
    padding: 6px 16px;
    border-top: 1px solid var(--border);
  }

  .ai input {
    flex: 1;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
    border-top: 1px solid var(--border);
    background: var(--surface-2);
  }

  .undo {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .small {
    font-size: 12px;
  }

  .overlay {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 35%);
    z-index: 30;
  }

  .dialog {
    width: min(460px, 90%);
    padding: 18px 20px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: 0 12px 40px rgb(0 0 0 / 25%);
  }

  .dialog h3 {
    margin: 0 0 10px;
    font-size: 16px;
  }

  .dialog dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 12px;
    margin: 0 0 10px;
  }

  .dialog dt {
    color: var(--text-muted);
  }

  .dialog dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .dialog p {
    margin: 0 0 12px;
  }

  .actions {
    display: flex;
    gap: 8px;
  }
</style>
