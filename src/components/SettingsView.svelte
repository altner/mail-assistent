<script lang="ts">
  import { onMount } from "svelte";
  import { api, errorText } from "../lib/api";
  import { refreshAiOverview } from "../lib/state.svelte";
  import type { AiSettingsView, Category, Preset, Tag } from "../lib/types";

  const presets: { id: Preset; label: string; url: string }[] = [
    { id: "ollama", label: "Ollama", url: "http://localhost:11434" },
    { id: "omlx", label: "oMLX", url: "http://127.0.0.1:8000" },
    { id: "splash", label: "Splash", url: "http://127.0.0.1:8000" },
    { id: "custom", label: "Eigener Server", url: "" },
  ];

  // --- KI-Server ---
  let saved = $state<AiSettingsView | null>(null);
  let preset = $state<Preset>("ollama");
  let url = $state("");
  let model = $state("");
  let apiKey = $state("");
  let removeKey = $state(false);
  let models = $state<string[]>([]);
  let aiStatus = $state<{ ok: boolean; text: string } | null>(null);
  let testing = $state(false);
  let savingAi = $state(false);

  function applySaved(s: AiSettingsView) {
    saved = s;
    preset = s.preset;
    url = s.url;
    model = s.model ?? "";
    apiKey = "";
    removeKey = false;
  }

  function changePreset(e: Event) {
    const next = (e.currentTarget as HTMLSelectElement).value as Preset;
    const previousDefault = presets.find((p) => p.id === preset)?.url ?? "";
    // URL nur ersetzen, wenn sie noch der Vorgabe des alten Servers entsprach.
    if (url.trim() === "" || url.trim() === previousDefault) {
      url = presets.find((p) => p.id === next)?.url ?? "";
    }
    preset = next;
    models = [];
    aiStatus = null;
  }

  /** Grobe Anzeige im Formular; maßgeblich ist die Prüfung im Rust-Kern. */
  const looksLocal = $derived.by(() => {
    try {
      const host = new URL(url.trim()).hostname.replace(/^\[|\]$/g, "").toLowerCase();
      return host === "localhost" || host.endsWith(".localhost") || host.startsWith("127.") || host === "::1";
    } catch {
      return true;
    }
  });

  async function testConnection() {
    testing = true;
    aiStatus = null;
    try {
      models = await api.testAi(url, removeKey ? undefined : apiKey);
      aiStatus = {
        ok: true,
        text: models.length
          ? `Verbunden, ${models.length} ${models.length === 1 ? "Modell" : "Modelle"} verfügbar.`
          : "Verbunden, aber der Server meldet keine Modelle.",
      };
      if (!model && models.length === 1) model = models[0];
    } catch (e) {
      models = [];
      aiStatus = { ok: false, text: errorText(e) };
    } finally {
      testing = false;
    }
  }

  async function saveAi(e: SubmitEvent) {
    e.preventDefault();
    savingAi = true;
    aiStatus = null;
    try {
      const key = removeKey ? "" : apiKey.trim() || undefined;
      applySaved(
        await api.saveAiSettings({ preset, url: url.trim(), model: model.trim() || null }, key),
      );
      aiStatus = { ok: true, text: "Gespeichert." };
    } catch (err) {
      aiStatus = { ok: false, text: errorText(err) };
    } finally {
      // Schlüssel nie im UI-Zustand behalten.
      apiKey = "";
      savingAi = false;
    }
  }

  // --- Kategorien ---
  let categories = $state<Category[]>([]);
  let editing = $state<number | null>(null);
  let editName = $state("");
  let editDescription = $state("");
  let newName = $state("");
  let newDescription = $state("");
  let categoryError = $state<string | null>(null);

  async function loadCategories() {
    try {
      categories = await api.listCategories();
      categoryError = null;
    } catch (e) {
      categoryError = errorText(e);
    }
  }

  function startEdit(c: Category) {
    editing = c.id;
    editName = c.name;
    editDescription = c.description ?? "";
  }

  async function saveEdit(e: SubmitEvent) {
    e.preventDefault();
    if (editing === null) return;
    try {
      await api.updateCategory(editing, { name: editName, description: editDescription || null });
      editing = null;
      await loadCategories();
    } catch (err) {
      categoryError = errorText(err);
    }
  }

  async function addCategory(e: SubmitEvent) {
    e.preventDefault();
    try {
      await api.createCategory({ name: newName, description: newDescription || null });
      newName = newDescription = "";
      await loadCategories();
    } catch (err) {
      categoryError = errorText(err);
    }
  }

  async function removeCategory(c: Category) {
    if (!confirm(`Kategorie „${c.name}“ löschen? Offene Vorschläge dafür werden verworfen.`)) return;
    try {
      await api.deleteCategory(c.id);
      await loadCategories();
      await refreshAiOverview();
    } catch (err) {
      categoryError = errorText(err);
    }
  }

  // --- Tags ---
  let tags = $state<Tag[]>([]);
  let newTag = $state("");
  let tagError = $state<string | null>(null);

  async function loadTags() {
    try {
      tags = await api.listTags();
      tagError = null;
    } catch (e) {
      tagError = errorText(e);
    }
  }

  async function addTag(e: SubmitEvent) {
    e.preventDefault();
    try {
      await api.createTag(newTag);
      newTag = "";
      await loadTags();
    } catch (err) {
      tagError = errorText(err);
    }
  }

  async function renameTag(t: Tag) {
    const name = prompt("Neuer Name für den Tag", t.name);
    if (name === null || name.trim() === t.name) return;
    try {
      await api.renameTag(t.id, name);
      await loadTags();
    } catch (err) {
      tagError = errorText(err);
    }
  }

  async function removeTag(t: Tag) {
    if (!confirm(`Tag „${t.name}“ löschen? Er wird von allen Mails entfernt.`)) return;
    try {
      await api.deleteTag(t.id);
      await loadTags();
    } catch (err) {
      tagError = errorText(err);
    }
  }

  // --- Senden ---
  let sendDelay = $state(0);
  let sendStatus = $state<string | null>(null);
  const delays = [
    { v: 0, label: "Sofort senden" },
    { v: 10, label: "10 Sekunden" },
    { v: 30, label: "30 Sekunden" },
    { v: 60, label: "60 Sekunden" },
  ];

  async function changeDelay(e: Event) {
    const v = Number((e.currentTarget as HTMLSelectElement).value);
    try {
      await api.setSendDelay(v);
      sendDelay = v;
      sendStatus = "Gespeichert.";
    } catch (err) {
      sendStatus = errorText(err);
    }
  }

  onMount(() => {
    api.getAiSettings().then(applySaved, (e) => (aiStatus = { ok: false, text: errorText(e) }));
    api.getSendDelay().then((d) => (sendDelay = d), () => {});
    loadCategories();
    loadTags();
  });
</script>

<section class="settings">
  <form class="card" onsubmit={saveAi}>
    <h2>KI-Server</h2>
    <p class="muted small">
      Ein lokaler Server mit Anthropic-Schnittstelle (<code>/v1/messages</code>). Die KI macht nur
      Vorschläge, die du im Review bestätigst.
    </p>
    <div class="row">
      <label>
        Server
        <select value={preset} onchange={changePreset}>
          {#each presets as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
        </select>
      </label>
      <label class="grow">
        URL
        <input bind:value={url} placeholder="http://127.0.0.1:8000" required autocomplete="off" />
      </label>
    </div>
    {#if !looksLocal}
      <p class="warning small">
        Dieser Server läuft nicht auf diesem Rechner. Mail-Inhalte werden dann über das Netz
        gesendet.
      </p>
    {/if}
    <label>
      API-Schlüssel
      <input
        type="password"
        bind:value={apiKey}
        disabled={removeKey}
        autocomplete="off"
        placeholder={saved?.hasApiKey ? "gespeichert, leer lassen für unverändert" : "optional (oMLX verlangt einen)"}
      />
    </label>
    {#if saved?.hasApiKey}
      <label class="check small">
        <input type="checkbox" bind:checked={removeKey} /> gespeicherten Schlüssel entfernen
      </label>
    {/if}
    <div class="row end">
      <label class="grow">
        Modell
        <input bind:value={model} list="ai-models" placeholder="erst „Verbindung testen“" autocomplete="off" />
        <datalist id="ai-models">
          {#each models as m (m)}<option value={m}></option>{/each}
        </datalist>
      </label>
      <button type="button" onclick={testConnection} disabled={testing || !url.trim()}>
        {testing ? "Teste …" : "Verbindung testen"}
      </button>
    </div>
    {#if models.length > 1}
      <div class="models small">
        {#each models as m (m)}
          <button type="button" class:chosen={m === model} onclick={() => (model = m)}>{m}</button>
        {/each}
      </div>
    {/if}
    {#if aiStatus}<p class="small" class:error={!aiStatus.ok} class:ok={aiStatus.ok}>{aiStatus.text}</p>{/if}
    <p class="muted small">Der API-Schlüssel liegt nur im Schlüsselbund des Systems.</p>
    <div><button class="primary" type="submit" disabled={savingAi}>{savingAi ? "Speichere …" : "Speichern"}</button></div>
  </form>

  <div class="card">
    <h2>Senden</h2>
    <p class="muted small">
      Jede Mail wird vor dem Senden in einem Dialog bestätigt. Zusätzlich kann eine Wartezeit mit
      „Rückgängig“ gelten; in der Zeit ist die Mail noch nicht beim Server.
    </p>
    <label>
      Wartezeit nach „Jetzt senden“
      <select value={sendDelay} onchange={changeDelay}>
        {#each delays as d (d.v)}<option value={d.v}>{d.label}</option>{/each}
      </select>
    </label>
    {#if sendStatus}<p class="muted small">{sendStatus}</p>{/if}
    <p class="muted small">
      Gesendete Mails werden nur lokal unter „Gesendet (lokal)“ abgelegt. Am Server-Postfach
      ändert die App nichts.
    </p>
  </div>

  <div class="card">
    <h2>Kategorien</h2>
    <p class="muted small">
      Jede Mail bekommt höchstens eine bestätigte Kategorie. Die Beschreibung hilft der KI beim
      Zuordnen. Die Reihenfolge bestimmt die Tasten 1–9 im Review.
    </p>
    {#if categoryError}<p class="error small">{categoryError}</p>{/if}
    <ol class="categories">
      {#each categories as c, i (c.id)}
        <li>
          {#if editing === c.id}
            <form class="edit" onsubmit={saveEdit}>
              <input bind:value={editName} required aria-label="Name" />
              <input bind:value={editDescription} placeholder="Beschreibung für die KI" aria-label="Beschreibung" />
              <div class="actions">
                <button class="primary" type="submit">Speichern</button>
                <button type="button" onclick={() => (editing = null)}>Abbrechen</button>
              </div>
            </form>
          {:else}
            <span class="key">{i < 9 ? i + 1 : ""}</span>
            <div class="text">
              <strong>{c.name}</strong>
              {#if c.confirmedCount}<span class="muted small"> · {c.confirmedCount} Mails</span>{/if}
              {#if c.description}<div class="muted small">{c.description}</div>{/if}
            </div>
            <div class="actions">
              <button onclick={() => startEdit(c)}>Bearbeiten</button>
              <button onclick={() => removeCategory(c)}>Löschen</button>
            </div>
          {/if}
        </li>
      {/each}
    </ol>
    <form class="add" onsubmit={addCategory}>
      <input bind:value={newName} placeholder="Neue Kategorie" required aria-label="Name der neuen Kategorie" />
      <input bind:value={newDescription} placeholder="Beschreibung (optional)" aria-label="Beschreibung der neuen Kategorie" />
      <button type="submit">Anlegen</button>
    </form>
  </div>

  <div class="card">
    <h2>Tags</h2>
    <p class="muted small">
      Zusätzliche Stichworte, beliebig viele pro Mail. Die KI wählt nur aus dieser Liste (höchstens
      drei) und legt keine neuen an.
    </p>
    {#if tagError}<p class="error small">{tagError}</p>{/if}
    {#if tags.length === 0}<p class="muted small">Noch keine Tags.</p>{/if}
    <ul class="tags">
      {#each tags as t (t.id)}
        <li>
          <button class="tag" title="Umbenennen" onclick={() => renameTag(t)}>{t.name}</button>
          <button class="remove" aria-label="Tag {t.name} löschen" onclick={() => removeTag(t)}>×</button>
        </li>
      {/each}
    </ul>
    <form class="add" onsubmit={addTag}>
      <input bind:value={newTag} placeholder="Neuer Tag" required aria-label="Name des neuen Tags" />
      <button type="submit">Anlegen</button>
    </form>
  </div>
</section>

<style>
  .settings {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
    gap: 20px;
    align-items: start;
    height: 100%;
    overflow-y: auto;
    padding: 20px 28px;
  }

  .card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 14px 16px;
    display: grid;
    gap: 10px;
    align-content: start;
  }

  h2 {
    margin: 0;
    font-size: 16px;
  }

  p {
    margin: 0;
  }

  label {
    display: grid;
    gap: 4px;
  }

  label.check {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .row {
    display: flex;
    gap: 8px;
  }

  .row.end {
    align-items: end;
  }

  .grow {
    flex: 1;
    min-width: 0;
  }

  .small {
    font-size: 12px;
  }

  .ok {
    color: var(--ok);
  }

  .warning {
    color: var(--danger);
  }

  .models {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .models button {
    padding: 2px 8px;
    font-size: 12px;
  }

  .models button.chosen {
    background: var(--selected);
    border-color: var(--accent);
  }

  code {
    font-family: var(--mono);
    font-size: 12px;
  }

  .categories,
  .tags {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .categories li {
    display: grid;
    grid-template-columns: 18px 1fr auto;
    gap: 8px;
    align-items: start;
    padding: 8px 0;
    border-top: 1px solid var(--border);
  }

  .key {
    color: var(--text-muted);
    font-family: var(--mono);
    font-size: 12px;
    padding-top: 2px;
  }

  .actions {
    display: flex;
    gap: 6px;
  }

  .actions button {
    padding: 2px 8px;
    font-size: 12px;
  }

  .edit {
    grid-column: 1 / -1;
    display: grid;
    gap: 6px;
  }

  .add {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .add input {
    flex: 1 1 140px;
    min-width: 0;
  }

  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .tags li {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 12px;
    overflow: hidden;
  }

  .tags button {
    border: 0;
    border-radius: 0;
    padding: 2px 8px;
    font-size: 12px;
  }

  .tags .remove {
    border-left: 1px solid var(--border);
    color: var(--text-muted);
  }
</style>
