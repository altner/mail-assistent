import { api, errorText } from "./api";
import type { Account, AiOverview, RunProgress, SyncProgress, SyncSummary } from "./types";

// App-weiter Zustand (Svelte-5-Runes).
export const store = $state({
  accounts: [] as Account[],
  accountsError: null as string | null,
  progress: {} as Record<number, SyncProgress>,
  lastSummary: {} as Record<number, SyncSummary>,
  /** wird nach jedem Abruf erhöht, damit Listen neu laden */
  dataVersion: 0,
  /** Stand des Vorschlagslaufs (bleibt beim Reiterwechsel erhalten) */
  aiProgress: null as RunProgress | null,
  aiOverview: null as AiOverview | null,
  /** wird nach jedem gespeicherten Vorschlag bzw. jeder Entscheidung erhöht */
  reviewVersion: 0,
});

export async function refreshAccounts() {
  try {
    store.accounts = await api.listAccounts();
    store.accountsError = null;
  } catch (e) {
    store.accountsError = errorText(e);
  }
}

export function applyProgress(p: SyncProgress) {
  store.progress[p.accountId] = p;
  // Nach jedem Block neu laden, damit neue Mails sofort sichtbar sind.
  if (p.phase === "folder" && p.done > 0) store.dataVersion++;
}

export async function syncAccount(accountId: number) {
  try {
    store.lastSummary[accountId] = await api.syncAccount(accountId);
  } catch (e) {
    // Fehler kommt zusätzlich als Fortschrittsereignis „failed“ an.
    const prev = store.progress[accountId];
    if (!prev || prev.phase !== "failed") {
      store.progress[accountId] = {
        accountId,
        phase: "failed",
        folder: null,
        folderIndex: 0,
        folderCount: 0,
        done: 0,
        total: 0,
        stats: { inserted: 0, duplicates: 0, remapped: 0, unparsable: 0 },
        error: errorText(e),
      };
    }
  } finally {
    store.dataVersion++;
    await refreshAccounts();
  }
}

export function isSyncing(accountId: number): boolean {
  const p = store.progress[accountId];
  return !!p && (p.phase === "connecting" || p.phase === "folder");
}

export async function refreshAiOverview() {
  try {
    store.aiOverview = await api.aiOverview();
  } catch {
    // Übersicht ist nur eine Anzeige; Fehler zeigt die Review-Ansicht selbst.
  }
}

export function applyAiProgress(p: RunProgress) {
  const stored = store.aiProgress?.stored ?? 0;
  store.aiProgress = p;
  if (p.stored !== stored || p.phase !== "running") {
    store.reviewVersion++;
    refreshAiOverview();
  }
}

export function isSuggesting(): boolean {
  return store.aiProgress?.phase === "running";
}

export async function startSuggestions(limit = 50) {
  store.aiProgress = {
    phase: "running",
    done: 0,
    total: 0,
    stored: 0,
    unreadable: 0,
    current: null,
    error: null,
  };
  try {
    store.aiProgress = await api.startSuggestions(limit);
  } catch (e) {
    store.aiProgress = { ...store.aiProgress, phase: "failed", current: null, error: errorText(e) };
  } finally {
    store.reviewVersion++;
    await refreshAiOverview();
  }
}
