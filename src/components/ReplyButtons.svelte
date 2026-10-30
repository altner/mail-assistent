<script lang="ts">
  import { errorText } from "../lib/api";
  import { composeFrom } from "../lib/state.svelte";
  import type { DraftKind } from "../lib/types";

  let { messageId }: { messageId: number } = $props();
  let error = $state<string | null>(null);
  let busy = $state(false);

  async function go(kind: DraftKind) {
    busy = true;
    error = null;
    try {
      await composeFrom(messageId, kind);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="buttons">
  <button disabled={busy} onclick={() => go("reply")}>Antworten</button>
  <button disabled={busy} onclick={() => go("reply_all")}>Allen antworten</button>
  <button disabled={busy} onclick={() => go("forward")}>Weiterleiten</button>
  {#if error}<span class="error small">{error}</span>{/if}
</div>

<style>
  .buttons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  button {
    padding: 3px 10px;
    font-size: 12px;
    white-space: nowrap;
  }

  .small {
    font-size: 12px;
  }
</style>
