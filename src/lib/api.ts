import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Account,
  AiOverview,
  AiSettings,
  AiSettingsView,
  AppInfo,
  Category,
  CategoryInput,
  Decision,
  MessageDetail,
  MessagePage,
  NewAccount,
  ReviewPage,
  RunProgress,
  SyncProgress,
  SyncSummary,
  Tag,
  ThreadDetail,
  ThreadPage,
} from "./types";

// Fehler aus Rust kommen als Text an; invoke() wirft sie als string.
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listAccounts: () => invoke<Account[]>("list_accounts"),
  createAccount: (account: NewAccount, password: string) =>
    invoke<Account>("create_account", { account, password }),
  syncAccount: (accountId: number) => invoke<SyncSummary>("sync_account", { accountId }),
  listMessages: (accountId: number | null, limit = 200, offset = 0) =>
    invoke<MessagePage>("list_messages", { accountId, limit, offset }),
  getMessage: (id: number) => invoke<MessageDetail>("get_message", { id }),
  listThreads: (accountId: number | null, limit = 200, offset = 0) =>
    invoke<ThreadPage>("list_threads", { accountId, limit, offset }),
  getThread: (id: number) => invoke<ThreadDetail>("get_thread", { id }),

  getAiSettings: () => invoke<AiSettingsView>("get_ai_settings"),
  /** apiKey: undefined = unverändert, "" = löschen */
  saveAiSettings: (settings: AiSettings, apiKey?: string) =>
    invoke<AiSettingsView>("save_ai_settings", { settings, apiKey: apiKey ?? null }),
  testAi: (url: string, apiKey?: string) =>
    invoke<string[]>("test_ai", { url, apiKey: apiKey || null }),

  listCategories: () => invoke<Category[]>("list_categories"),
  createCategory: (input: CategoryInput) => invoke<number>("create_category", { input }),
  updateCategory: (id: number, input: CategoryInput) =>
    invoke<void>("update_category", { id, input }),
  deleteCategory: (id: number) => invoke<void>("delete_category", { id }),
  listTags: () => invoke<Tag[]>("list_tags"),
  createTag: (name: string) => invoke<number>("create_tag", { name }),
  renameTag: (id: number, name: string) => invoke<void>("rename_tag", { id, name }),
  deleteTag: (id: number) => invoke<void>("delete_tag", { id }),

  aiOverview: () => invoke<AiOverview>("ai_overview"),
  startSuggestions: (limit = 50) => invoke<RunProgress>("start_suggestions", { limit }),
  cancelSuggestions: () => invoke<void>("cancel_suggestions"),
  listReview: (limit = 50, offset = 0) => invoke<ReviewPage>("list_review", { limit, offset }),
  decideReview: (id: number, decision: Decision, tagIds: number[]) =>
    invoke<void>("decide_review", { id, input: { decision, tagIds } }),
};

export function onAiProgress(handler: (p: RunProgress) => void): Promise<UnlistenFn> {
  return listen<RunProgress>("ai://progress", (event) => handler(event.payload));
}

export function onSyncProgress(handler: (p: SyncProgress) => void): Promise<UnlistenFn> {
  return listen<SyncProgress>("sync://progress", (event) => handler(event.payload));
}
