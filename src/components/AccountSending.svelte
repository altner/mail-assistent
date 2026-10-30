<script lang="ts">
  import { untrack } from "svelte";
  import { api, errorText } from "../lib/api";
  import { refreshAccounts } from "../lib/state.svelte";
  import type { Account, Identity } from "../lib/types";

  // SMTP-Server und Absenderadressen eines Kontos. Zugangsdaten sind die des
  // IMAP-Kontos aus dem Schlüsselbund; hier wird nichts davon angezeigt.

  let { account }: { account: Account } = $props();

  let open = $state(false);
  let smtpHost = $state("");
  let smtpPort = $state(465);
  let smtpStatus = $state<{ ok: boolean; text: string } | null>(null);
  let testing = $state(false);
  let identities = $state<Identity[]>([]);
  let idError = $state<string | null>(null);
  let editing = $state<number | null>(null);
  let form = $state({ name: "", email: "", signatureText: "" });

  $effect(() => {
    const a = account;
    untrack(() => {
      smtpHost = a.smtpHost ?? guessSmtp(a.imapHost);
      smtpPort = a.smtpPort || 465;
    });
  });

  function guessSmtp(imap: string): string {
    return imap.replace(/^imap\./, "smtp.");
  }

  async function loadIdentities() {
    try {
      identities = await api.listIdentities(account.id);
      idError = null;
    } catch (e) {
      idError = errorText(e);
    }
  }

  $effect(() => {
    if (open) untrack(loadIdentities);
  });

  async function saveSmtp(e: SubmitEvent) {
    e.preventDefault();
    smtpStatus = null;
    try {
      await api.updateAccountSmtp(account.id, smtpHost, smtpPort);
      await refreshAccounts();
      smtpStatus = { ok: true, text: "Gespeichert." };
    } catch (err) {
      smtpStatus = { ok: false, text: errorText(err) };
    }
  }

  async function testSmtp() {
    testing = true;
    smtpStatus = null;
    try {
      await api.testSmtp(account.id, smtpHost, smtpPort);
      smtpStatus = { ok: true, text: "Anmeldung am SMTP-Server erfolgreich. Es wurde nichts gesendet." };
    } catch (err) {
      smtpStatus = { ok: false, text: errorText(err) };
    } finally {
      testing = false;
    }
  }

  function startEdit(i: Identity | null) {
    editing = i?.id ?? 0;
    form = { name: i?.name ?? "", email: i?.email ?? "", signatureText: i?.signatureText ?? "" };
  }

  async function saveIdentity(e: SubmitEvent) {
    e.preventDefault();
    const input = {
      name: form.name,
      email: form.email,
      signatureText: form.signatureText || null,
      signatureHtml: null,
    };
    try {
      if (editing === 0) await api.createIdentity(account.id, input);
      else if (editing !== null) await api.updateIdentity(editing, input);
      editing = null;
      await loadIdentities();
    } catch (err) {
      idError = errorText(err);
    }
  }

  async function setDefault(i: Identity) {
    try {
      await api.setDefaultIdentity(i.id);
      await loadIdentities();
    } catch (err) {
      idError = errorText(err);
    }
  }

  async function remove(i: Identity) {
    if (!confirm(`Absenderadresse „${i.email}“ löschen?`)) return;
    try {
      await api.deleteIdentity(i.id);
      await loadIdentities();
    } catch (err) {
      idError = errorText(err);
    }
  }
</script>

<div class="sending">
  <button class="toggle" aria-expanded={open} onclick={() => (open = !open)}>
    {open ? "▾" : "▸"} Senden: {account.smtpHost ? `${account.smtpHost}:${account.smtpPort}` : "kein SMTP-Server"}
  </button>

  {#if open}
    <form class="smtp" onsubmit={saveSmtp}>
      <label class="grow">SMTP-Server <input bind:value={smtpHost} placeholder="smtp.example.org" autocomplete="off" /></label>
      <label class="port">Port <input type="number" bind:value={smtpPort} min="1" max="65535" /></label>
      <button type="submit" class="primary">Speichern</button>
      <button type="button" onclick={testSmtp} disabled={testing || !smtpHost.trim()}>{testing ? "Teste …" : "Anmeldung testen"}</button>
    </form>
    <p class="muted small">
      Nur implizites TLS (SMTPS, meist Port 465). Angemeldet wird mit Benutzername und Passwort
      des IMAP-Kontos.
    </p>
    {#if smtpStatus}<p class="small" class:ok={smtpStatus.ok} class:error={!smtpStatus.ok}>{smtpStatus.text}</p>{/if}

    <h4>Absenderadressen</h4>
    {#if idError}<p class="error small">{idError}</p>{/if}
    <ul>
      {#each identities as i (i.id)}
        <li>
          {#if editing === i.id}
            <form class="edit" onsubmit={saveIdentity}>
              <input bind:value={form.name} placeholder="Name" required aria-label="Name" />
              <input bind:value={form.email} placeholder="adresse@example.org" required aria-label="Adresse" />
              <textarea bind:value={form.signatureText} rows="3" placeholder="Signatur (optional)" aria-label="Signatur"></textarea>
              <div class="actions">
                <button class="primary" type="submit">Speichern</button>
                <button type="button" onclick={() => (editing = null)}>Abbrechen</button>
              </div>
            </form>
          {:else}
            <div class="text">
              <strong>{i.name}</strong> <span class="muted">&lt;{i.email}&gt;</span>
              {#if i.isDefault}<span class="badge">Standard</span>{/if}
              {#if i.signatureText}<div class="muted small sig">{i.signatureText}</div>{/if}
            </div>
            <div class="actions">
              {#if !i.isDefault}<button onclick={() => setDefault(i)}>Standard</button>{/if}
              <button onclick={() => startEdit(i)}>Bearbeiten</button>
              <button onclick={() => remove(i)}>Löschen</button>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
    {#if editing === 0}
      <form class="edit" onsubmit={saveIdentity}>
        <input bind:value={form.name} placeholder="Name" required aria-label="Name" />
        <input bind:value={form.email} placeholder="adresse@example.org" required aria-label="Adresse" />
        <textarea bind:value={form.signatureText} rows="3" placeholder="Signatur (optional)" aria-label="Signatur"></textarea>
        <div class="actions">
          <button class="primary" type="submit">Anlegen</button>
          <button type="button" onclick={() => (editing = null)}>Abbrechen</button>
        </div>
      </form>
    {:else}
      <button onclick={() => startEdit(null)}>Adresse hinzufügen</button>
    {/if}
  {/if}
</div>

<style>
  .sending {
    display: grid;
    gap: 6px;
    padding-top: 6px;
    border-top: 1px solid var(--border);
  }

  .toggle {
    justify-self: start;
    border: 0;
    background: transparent;
    padding: 2px 0;
    font-size: 13px;
    color: var(--text-muted);
  }

  .smtp {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: 8px;
  }

  .smtp label {
    display: grid;
    gap: 4px;
    font-size: 13px;
  }

  .grow {
    flex: 1;
    min-width: 160px;
  }

  .port {
    width: 90px;
  }

  h4 {
    margin: 8px 0 0;
    font-size: 13px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  li {
    display: flex;
    justify-content: space-between;
    align-items: start;
    gap: 8px;
    padding: 6px 0;
    border-top: 1px solid var(--border);
  }

  li form {
    width: 100%;
  }

  .edit {
    display: grid;
    gap: 6px;
  }

  .edit textarea {
    font: inherit;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 5px 8px;
    background: var(--surface);
    color: inherit;
  }

  .actions {
    display: flex;
    gap: 6px;
  }

  .actions button {
    padding: 2px 8px;
    font-size: 12px;
  }

  .badge {
    margin-left: 6px;
    padding: 0 6px;
    border-radius: 9px;
    background: var(--selected);
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
  }

  .sig {
    white-space: pre-wrap;
  }

  .small {
    font-size: 12px;
  }

  .ok {
    color: var(--ok);
  }
</style>
