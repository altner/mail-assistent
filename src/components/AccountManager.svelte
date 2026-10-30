<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorText } from "../lib/api";
  import { isSyncing, refreshAccounts, store, syncAccount } from "../lib/state.svelte";
  import type { AppInfo, SyncProgress } from "../lib/types";

  let name = $state("");
  let imapHost = $state("");
  let imapPort = $state(993);
  let username = $state("");
  let password = $state("");
  let saving = $state(false);
  let formError = $state<string | null>(null);
  let info = $state<AppInfo | null>(null);

  onMount(() => {
    api.appInfo().then((i) => (info = i));
  });

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    saving = true;
    formError = null;
    try {
      await api.createAccount({ name, imapHost, imapPort, username }, password);
      name = imapHost = username = "";
      imapPort = 993;
      await refreshAccounts();
    } catch (err) {
      formError = errorText(err);
    } finally {
      // Passwort nie im UI-Zustand behalten.
      password = "";
      saving = false;
    }
  }

  function progressText(p: SyncProgress): string {
    switch (p.phase) {
      case "connecting":
        return "Verbinde …";
      case "folder":
        return `Ordner ${p.folderIndex + 1}/${p.folderCount}: ${p.folder ?? ""}` +
          (p.total > 0 ? ` – ${p.done}/${p.total}` : "");
      case "done":
        return `Fertig: ${p.stats.inserted} neu, ${p.stats.duplicates} Duplikate übersprungen` +
          (p.stats.remapped ? `, ${p.stats.remapped} neu zugeordnet` : "");
      case "failed":
        return `Fehler: ${p.error ?? "unbekannt"}`;
    }
  }

  function percent(p: SyncProgress): number {
    if (p.phase === "done") return 100;
    if (p.folderCount === 0) return 0;
    const inFolder = p.total > 0 ? p.done / p.total : 0;
    return Math.round(((p.folderIndex + inFolder) / p.folderCount) * 100);
  }
</script>

<section class="accounts">
  <div class="column">
    <h2>Konten</h2>
    {#if store.accountsError}
      <p class="error">{store.accountsError}</p>
    {/if}
    {#if store.accounts.length === 0}
      <p class="muted">Noch keine Konten.</p>
    {/if}
    <ul>
      {#each store.accounts as account (account.id)}
        {@const p = store.progress[account.id]}
        {@const summary = store.lastSummary[account.id]}
        <li class="card">
          <div class="head">
            <div>
              <strong>{account.name}</strong>
              <div class="muted">{account.username} · {account.imapHost}:{account.imapPort}</div>
            </div>
            <button class="primary" disabled={isSyncing(account.id)} onclick={() => syncAccount(account.id)}>
              {isSyncing(account.id) ? "Ruft ab …" : "Abrufen"}
            </button>
          </div>
          <div class="muted">{account.messageCount.toLocaleString("de-DE")} Mails archiviert</div>
          {#if p}
            <progress max="100" value={percent(p)}></progress>
            <div class:error={p.phase === "failed"}>{progressText(p)}</div>
          {/if}
          {#if summary && !isSyncing(account.id)}
            {#if summary.skippedFolders.length}
              <div class="muted small">Übersprungen: {summary.skippedFolders.join(", ")}</div>
            {/if}
            {#if summary.folderErrors.length}
              <div class="error small">Ordnerfehler: {summary.folderErrors.join("; ")}</div>
            {/if}
          {/if}
        </li>
      {/each}
    </ul>
    {#if info}
      <p class="muted small">Daten: <code>{info.dataDir}</code></p>
    {/if}
  </div>

  <form class="column card" onsubmit={submit}>
    <h3>Neues Konto</h3>
    <label>Name <input bind:value={name} placeholder="z. B. Privat" required /></label>
    <div class="row">
      <label class="grow">IMAP-Server <input bind:value={imapHost} placeholder="imap.example.org" required autocomplete="off" /></label>
      <label class="port">Port <input type="number" bind:value={imapPort} min="1" max="65535" required /></label>
    </div>
    <label>Benutzername <input bind:value={username} required autocomplete="username" /></label>
    <label>
      Passwort / App-Passwort
      <input type="password" bind:value={password} required autocomplete="new-password" />
    </label>
    <p class="muted small">
      Verbindung nur per TLS (implizit, meist Port 993). Das Passwort wird ausschließlich im
      Schlüsselbund des Systems gespeichert. Der Server wird nur gelesen, nichts wird verändert.
    </p>
    {#if formError}<p class="error">{formError}</p>{/if}
    <button class="primary" type="submit" disabled={saving}>{saving ? "Speichere …" : "Konto anlegen"}</button>
  </form>
</section>

<style>
  .accounts {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(300px, 380px);
    gap: 24px;
    align-items: start;
    height: 100%;
    overflow-y: auto;
    padding: 20px 28px;
  }

  h2,
  h3 {
    margin: 0 0 12px;
    font-size: 18px;
  }

  h3 {
    font-size: 15px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 12px;
  }

  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 14px 16px;
    display: grid;
    gap: 6px;
  }

  .head {
    display: flex;
    justify-content: space-between;
    align-items: start;
    gap: 12px;
  }

  progress {
    width: 100%;
    accent-color: var(--accent);
  }

  form label {
    display: grid;
    gap: 4px;
  }

  .row {
    display: flex;
    gap: 8px;
  }

  .grow {
    flex: 1;
  }

  .port {
    width: 90px;
  }

  .small {
    font-size: 12px;
  }

  code {
    font-family: var(--mono);
    font-size: 12px;
    overflow-wrap: anywhere;
  }
</style>
