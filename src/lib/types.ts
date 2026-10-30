// Spiegelt die serialisierten Rust-Typen (camelCase).

export interface Account {
  id: number;
  name: string;
  imapHost: string;
  imapPort: number;
  username: string;
  createdAt: string;
  messageCount: number;
  smtpHost: string | null;
  smtpPort: number;
}

export interface NewAccount {
  name: string;
  imapHost: string;
  imapPort: number;
  username: string;
}

export interface MessageSummary {
  id: number;
  accountId: number;
  folderPath: string;
  folderName: string;
  fromName: string | null;
  fromAddr: string | null;
  subject: string | null;
  /** RFC 3339, UTC */
  date: string | null;
  isRead: boolean;
  threadId: number | null;
  /** bestätigte Kategorie, nie ein offener Vorschlag */
  category: string | null;
}

export interface MessageDetail extends MessageSummary {
  messageId: string | null;
  inReplyTo: string | null;
  /** JSON-Array von {name, address} */
  toAddrs: string | null;
  bodyText: string | null;
  rawPath: string;
}

export interface MessagePage {
  items: MessageSummary[];
  total: number;
}

/** Mail-Liste: einzelne Mails oder Konversationen */
export type ListMode = "messages" | "threads";

export interface ThreadSummary {
  id: number;
  accountId: number;
  /** Betreff ohne Re:/AW:/Fwd: */
  subject: string | null;
  lastDate: string | null;
  messageCount: number;
  unreadCount: number;
  /** Absender in der Reihenfolge ihres ersten Auftretens (höchstens 4) */
  participants: string[];
}

export interface ThreadDetail extends ThreadSummary {
  /** älteste zuerst */
  messages: MessageDetail[];
}

export interface ThreadPage {
  items: ThreadSummary[];
  total: number;
}

export interface IngestStats {
  inserted: number;
  duplicates: number;
  remapped: number;
  unparsable: number;
}

export type SyncPhase = "connecting" | "folder" | "done" | "failed";

export interface SyncProgress {
  accountId: number;
  phase: SyncPhase;
  folder: string | null;
  folderIndex: number;
  folderCount: number;
  done: number;
  total: number;
  stats: IngestStats;
  error: string | null;
}

export interface SyncSummary {
  accountId: number;
  folders: string[];
  skippedFolders: string[];
  folderErrors: string[];
  stats: IngestStats;
}

export interface AppInfo {
  dataDir: string;
}

export type Preset = "ollama" | "omlx" | "splash" | "custom";

export interface AiSettings {
  preset: Preset;
  url: string;
  model: string | null;
}

export interface AiSettingsView extends AiSettings {
  hasApiKey: boolean;
  /** Server läuft auf diesem Rechner */
  isLocal: boolean;
}

export interface Category {
  id: number;
  name: string;
  description: string | null;
  confirmedCount: number;
}

export interface CategoryInput {
  name: string;
  description: string | null;
}

export interface Tag {
  id: number;
  name: string;
}

export interface AiOverview {
  pending: number;
  candidates: number;
  running: boolean;
}

export type RunPhase = "running" | "done" | "cancelled" | "failed";

export interface RunProgress {
  phase: RunPhase;
  done: number;
  total: number;
  stored: number;
  unreadable: number;
  current: string | null;
  error: string | null;
}

export interface ReviewTag {
  id: number;
  name: string;
  source: "ai" | "user";
  status: "pending" | "accepted" | "rejected";
}

export interface ReviewItem {
  id: number;
  categoryId: number | null;
  categoryName: string | null;
  confidence: number | null;
  reason: string | null;
  model: string | null;
  tags: ReviewTag[];
  message: MessageDetail;
}

export interface ReviewPage {
  items: ReviewItem[];
  total: number;
}

export type Decision =
  | { kind: "accept" }
  | { kind: "change"; categoryId: number }
  | { kind: "reject" };

export interface Address {
  name: string | null;
  address: string | null;
}

export interface AttachmentInfo {
  part: number;
  filename: string;
  mimeType: string;
  size: number;
}

export interface MailBody {
  /** vollständiges Dokument für srcdoc, bereinigt, mit eigener CSP */
  html: string;
  isHtml: boolean;
  blockedImages: number;
  attachments: AttachmentInfo[];
}

export interface Identity {
  id: number;
  accountId: number;
  name: string;
  email: string;
  signatureText: string | null;
  signatureHtml: string | null;
  isDefault: boolean;
}

export interface IdentityInput {
  name: string;
  email: string;
  signatureText: string | null;
  signatureHtml: string | null;
}

export type DraftKind = "new" | "reply" | "reply_all" | "forward";

export interface Recipient {
  name: string | null;
  address: string;
}

export type DraftAttachment =
  | { source: "message"; messageId: number; part: number; name: string; mime: string; size: number }
  | { source: "file"; path: string; name: string; mime: string; size: number };

export interface Draft {
  id: number;
  accountId: number;
  identityId: number | null;
  inReplyToMessage: number | null;
  kind: DraftKind;
  to: Recipient[];
  cc: Recipient[];
  bcc: Recipient[];
  subject: string;
  bodyHtml: string;
  attachments: DraftAttachment[];
  createdAt: string;
  updatedAt: string;
}

export interface DraftUpdate {
  identityId: number | null;
  to: Recipient[];
  cc: Recipient[];
  bcc: Recipient[];
  subject: string;
  bodyHtml: string;
}

export interface Contact {
  id: number;
  email: string;
  name: string | null;
  source: "archive" | "manual";
  seenCount: number;
  lastSeen: string | null;
}

export interface ContactInput {
  email: string;
  name: string | null;
}

export type SendPhase = "waiting" | "sending" | "sent" | "cancelled" | "failed";

export interface SendStatus {
  draftId: number;
  phase: SendPhase;
  secondsLeft: number;
  error: string | null;
  messageId: number | null;
}
