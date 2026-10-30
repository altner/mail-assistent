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
  Contact,
  ContactInput,
  Decision,
  Draft,
  DraftKind,
  DraftUpdate,
  Identity,
  IdentityInput,
  MailBody,
  MessageDetail,
  MessagePage,
  NewAccount,
  ReviewPage,
  RunProgress,
  SendStatus,
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
  listMessages: (accountId: number | null, query = "", limit = 200, offset = 0) =>
    invoke<MessagePage>("list_messages", { accountId, query: query || null, limit, offset }),
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

  getMessageBody: (id: number, allowRemote = false) =>
    invoke<MailBody>("get_message_body", { id, allowRemote }),
  openLink: (url: string) => invoke<void>("open_link", { url }),
  /** null: Dialog abgebrochen */
  saveAttachment: (id: number, part: number) =>
    invoke<string | null>("save_attachment", { id, part }),

  updateAccountSmtp: (accountId: number, smtpHost: string, smtpPort: number) =>
    invoke<Account>("update_account_smtp", { accountId, smtpHost, smtpPort }),
  testSmtp: (accountId: number, smtpHost: string, smtpPort: number) =>
    invoke<void>("test_smtp", { accountId, smtpHost, smtpPort }),
  listIdentities: (accountId: number | null = null) =>
    invoke<Identity[]>("list_identities", { accountId }),
  createIdentity: (accountId: number, input: IdentityInput) =>
    invoke<number>("create_identity", { accountId, input }),
  updateIdentity: (id: number, input: IdentityInput) =>
    invoke<void>("update_identity", { id, input }),
  setDefaultIdentity: (id: number) => invoke<void>("set_default_identity", { id }),
  deleteIdentity: (id: number) => invoke<void>("delete_identity", { id }),

  prepareDraft: (messageId: number, kind: DraftKind) =>
    invoke<Draft>("prepare_draft", { messageId, kind }),
  newDraft: (accountId: number) => invoke<Draft>("new_draft", { accountId }),
  listDrafts: () => invoke<Draft[]>("list_drafts"),
  getDraft: (id: number) => invoke<Draft>("get_draft", { id }),
  updateDraft: (id: number, update: DraftUpdate) => invoke<void>("update_draft", { id, update }),
  deleteDraft: (id: number) => invoke<void>("delete_draft", { id }),
  /** null: Dialog abgebrochen */
  addDraftFile: (id: number) => invoke<Draft | null>("add_draft_file", { id }),
  removeDraftAttachment: (id: number, index: number) =>
    invoke<Draft>("remove_draft_attachment", { id, index }),

  suggestContacts: (query: string) => invoke<Contact[]>("suggest_contacts", { query }),
  listContacts: () => invoke<Contact[]>("list_contacts"),
  createContact: (input: ContactInput) => invoke<number>("create_contact", { input }),
  updateContact: (id: number, input: ContactInput) => invoke<void>("update_contact", { id, input }),
  deleteContact: (id: number) => invoke<void>("delete_contact", { id }),
  rebuildContacts: () => invoke<number>("rebuild_contacts"),

  getSendDelay: () => invoke<number>("get_send_delay"),
  setSendDelay: (seconds: number) => invoke<void>("set_send_delay", { seconds }),
  /** Nur nach dem Bestätigungsdialog aufrufen. Löst erst nach Versand/Abbruch auf. */
  sendDraft: (id: number) => invoke<SendStatus>("send_draft", { id }),
  cancelSend: (id: number) => invoke<boolean>("cancel_send", { id }),
  aiDraft: (id: number, hint: string) => invoke<string>("ai_draft", { id, hint: hint || null }),
};

export function onSendStatus(handler: (s: SendStatus) => void): Promise<UnlistenFn> {
  return listen<SendStatus>("send://status", (event) => handler(event.payload));
}

export function onAiProgress(handler: (p: RunProgress) => void): Promise<UnlistenFn> {
  return listen<RunProgress>("ai://progress", (event) => handler(event.payload));
}

export function onSyncProgress(handler: (p: SyncProgress) => void): Promise<UnlistenFn> {
  return listen<SyncProgress>("sync://progress", (event) => handler(event.payload));
}
