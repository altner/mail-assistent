<script lang="ts">
  import { api } from "../lib/api";
  import type { Contact, Recipient } from "../lib/types";

  let {
    label,
    recipients = $bindable<Recipient[]>([]),
    onchange,
  }: { label: string; recipients?: Recipient[]; onchange?: () => void } = $props();

  let input = $state("");
  let suggestions = $state<Contact[]>([]);
  let active = $state(0);
  let open = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  const emailRe = /^[^\s<>@,]+@[^\s<>@,]+\.[^\s<>@,]+$/;

  function search() {
    clearTimeout(timer);
    const q = input.trim();
    if (!q) {
      suggestions = [];
      open = false;
      return;
    }
    timer = setTimeout(async () => {
      try {
        const hits = await api.suggestContacts(q);
        suggestions = hits.filter((c) => !recipients.some((r) => r.address === c.email));
        active = 0;
        open = suggestions.length > 0;
      } catch {
        suggestions = [];
      }
    }, 120);
  }

  function add(r: Recipient): boolean {
    const address = r.address.trim().toLowerCase();
    if (!emailRe.test(address)) return false;
    if (!recipients.some((x) => x.address === address)) {
      recipients = [...recipients, { name: r.name, address }];
      onchange?.();
    }
    input = "";
    suggestions = [];
    open = false;
    return true;
  }

  /** „Name <adresse>“ oder nur Adresse aus dem Eingabefeld übernehmen. */
  function commitInput(): boolean {
    const raw = input.trim().replace(/[,;]$/, "");
    if (!raw) return true;
    const m = raw.match(/^(.*?)\s*<([^>]+)>$/);
    const r: Recipient = m
      ? { name: m[1].replace(/^"|"$/g, "").trim() || null, address: m[2] }
      : { name: null, address: raw };
    return add(r);
  }

  function remove(i: number) {
    recipients = recipients.filter((_, k) => k !== i);
    onchange?.();
  }

  function onKeydown(e: KeyboardEvent) {
    if (open && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      e.preventDefault();
      active = (active + (e.key === "ArrowDown" ? 1 : suggestions.length - 1)) % suggestions.length;
    } else if (e.key === "Enter" || e.key === "Tab" || e.key === "," || e.key === ";") {
      if (!input.trim()) return;
      e.preventDefault();
      if (open && suggestions[active]) {
        const c = suggestions[active];
        add({ name: c.name, address: c.email });
      } else {
        commitInput();
      }
    } else if (e.key === "Backspace" && !input && recipients.length) {
      remove(recipients.length - 1);
    } else if (e.key === "Escape") {
      open = false;
    }
  }

  function onBlur() {
    // Vorschlagsklick abwarten, dann Rest übernehmen.
    setTimeout(() => {
      open = false;
      commitInput();
    }, 150);
  }

  function invalid(): boolean {
    const raw = input.trim();
    return raw.length > 0 && !emailRe.test(raw.match(/<([^>]+)>$/)?.[1] ?? raw);
  }
</script>

<div class="field">
  <span class="label muted">{label}</span>
  <div class="chips">
    {#each recipients as r, i (r.address)}
      <span class="chip" title={r.address}>
        {r.name ?? r.address}
        <button type="button" class="x" aria-label="{r.address} entfernen" onclick={() => remove(i)}>×</button>
      </span>
    {/each}
    <input
      bind:value={input}
      oninput={search}
      onkeydown={onKeydown}
      onblur={onBlur}
      class:invalid={invalid()}
      placeholder={recipients.length ? "" : "Name oder Adresse"}
      autocomplete="off"
      aria-label={label}
    />
  </div>
  {#if open}
    <ul class="suggest" role="listbox">
      {#each suggestions as c, i (c.id)}
        <li>
          <button
            type="button"
            role="option"
            aria-selected={i === active}
            class:active={i === active}
            onmousedown={(e) => e.preventDefault()}
            onclick={() => add({ name: c.name, address: c.email })}
          >
            {#if c.name}<strong>{c.name}</strong> {/if}<span class="muted">{c.email}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .field {
    position: relative;
    display: grid;
    grid-template-columns: 56px 1fr;
    align-items: start;
    gap: 6px;
    padding: 4px 0;
    border-bottom: 1px solid var(--border);
  }

  .label {
    padding-top: 4px;
    font-size: 13px;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    min-width: 0;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 1px 4px 1px 8px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: var(--surface-2);
    font-size: 13px;
  }

  .x {
    border: 0;
    background: transparent;
    padding: 0 4px;
    color: var(--text-muted);
    line-height: 1;
  }

  input {
    flex: 1;
    min-width: 140px;
    border: 0;
    background: transparent;
    padding: 3px 4px;
  }

  input.invalid {
    color: var(--danger);
  }

  .suggest {
    position: absolute;
    top: 100%;
    left: 56px;
    right: 0;
    z-index: 5;
    list-style: none;
    margin: 2px 0 0;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: 0 6px 20px rgb(0 0 0 / 12%);
  }

  .suggest button {
    width: 100%;
    text-align: left;
    border: 0;
    background: transparent;
    padding: 5px 8px;
    border-radius: 4px;
  }

  .suggest button.active,
  .suggest button:hover {
    background: var(--selected);
  }
</style>
