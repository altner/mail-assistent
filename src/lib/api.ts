import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Account,
  AppInfo,
  MessageDetail,
  MessagePage,
  NewAccount,
  SyncProgress,
  SyncSummary,
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
};

export function onSyncProgress(handler: (p: SyncProgress) => void): Promise<UnlistenFn> {
  return listen<SyncProgress>("sync://progress", (event) => handler(event.payload));
}
