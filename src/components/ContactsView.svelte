<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorText } from "../lib/api";
  import { shortDate } from "../lib/format";
  import type { Contact } from "../lib/types";

  let contacts = $state<Contact[]>([]);
  let filter = $state("");
  let error = $state<string | null>(null);
  let editing = $state<number | null>(null);
  let editName = $state("");
  let editEmail = $state("");
  let newName = $state("");
  let newEmail = $state("");
  let rebuilding = $state(false);

  async function load() {
    try {
      contacts = await api.listContacts();
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  onMount(load);

  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return contacts;
    return contacts.filter((c) => c.email.includes(q) || (c.name ?? "").toLowerCase().includes(q));
  });

  function startEdit(c: Contact) {
    editing = c.id;
    editName = c.name ?? "";
    editEmail = c.email;
  }

  async function saveEdit(e: SubmitEvent) {
    e.preventDefault();
    if (editing === null) return;
    try {
      await api.updateContact(editing, { email: editEmail, name: editName || null });
      editing = null;
      await load();
    } catch (err) {
      error = errorText(err);
    }
  }

  async function add(e: SubmitEvent) {
    e.preventDefault();
    try {
      await api.createContact({ email: newEmail, name: newName || null });
      newName = newEmail = "";
      await load();
    } catch (err) {
      error = errorText(err);
    }
  }

  async function remove(c: Contact) {
    if (!confirm(`Kontakt „${c.name ?? c.email}“ löschen?`)) return;
    try {
      await api.deleteContact(c.id);
      await load();
    } catch (err) {
      error = errorText(err);
    }
  }

  async function rebuild() {
    rebuilding = true;
    try {
      await api.rebuildContacts();
      await load();
    } catch (err) {
      error = errorText(err);
    } finally {
      rebuilding = false;
    }
  }
</script>

<section class="contacts">
  <header>
    <h2>Kontakte <span class="muted small">{contacts.length.toLocaleString("de-DE")}</span></h2>
    <input type="search" bind:value={filter} placeholder="Filtern" aria-label="Kontakte filtern" />
    <button onclick={rebuild} disabled={rebuilding} title="Archiv-Kontakte neu zählen; manuelle bleiben">
      {rebuilding ? "Zähle …" : "Aus Archiv neu aufbauen"}
    </button>
  </header>
  <p class="muted small">
    Gesammelt aus Absendern und Empfängern des Archivs, sortiert nach Name. Bearbeitete und neue
    Einträge gelten als manuell und werden vom Archiv nicht mehr überschrieben.
  </p>
  {#if error}<p class="error">{error}</p>{/if}

  <form class="add" onsubmit={add}>
    <input bind:value={newName} placeholder="Name (optional)" aria-label="Name" />
    <input bind:value={newEmail} placeholder="adresse@example.org" required aria-label="Adresse" />
    <button type="submit">Anlegen</button>
  </form>

  <ul>
    {#each shown as c (c.id)}
      <li>
        {#if editing === c.id}
          <form class="edit" onsubmit={saveEdit}>
            <input bind:value={editName} placeholder="Name" aria-label="Name" />
            <input bind:value={editEmail} required aria-label="Adresse" />
            <button class="primary" type="submit">Speichern</button>
            <button type="button" onclick={() => (editing = null)}>Abbrechen</button>
          </form>
        {:else}
          <div class="text">
            <strong>{c.name ?? c.email}</strong>
            {#if c.name}<span class="muted"> {c.email}</span>{/if}
            <div class="muted small">
              {c.source === "manual" ? "manuell" : `${c.seenCount}× im Archiv`}
              {#if c.lastSeen} · zuletzt {shortDate(c.lastSeen)}{/if}
            </div>
          </div>
          <div class="actions">
            <button onclick={() => startEdit(c)}>Bearbeiten</button>
            <button onclick={() => remove(c)}>Löschen</button>
          </div>
        {/if}
      </li>
    {/each}
  </ul>
</section>

<style>
  .contacts {
    height: 100%;
    overflow-y: auto;
    padding: 20px 28px;
    max-width: 820px;
  }

  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    margin-bottom: 6px;
  }

  h2 {
    margin: 0;
    font-size: 18px;
  }

  header input {
    flex: 1;
    min-width: 160px;
  }

  p {
    margin: 0 0 12px;
  }

  .add,
  .edit {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .add {
    margin-bottom: 12px;
  }

  .add input,
  .edit input {
    flex: 1 1 160px;
    min-width: 0;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 8px 0;
    border-top: 1px solid var(--border);
  }

  li form {
    width: 100%;
  }

  .actions {
    display: flex;
    gap: 6px;
  }

  .actions button {
    padding: 2px 8px;
    font-size: 12px;
  }

  .small {
    font-size: 12px;
  }
</style>
