// Spiegelt die serialisierten Rust-Typen (camelCase).

export interface Account {
  id: number;
  name: string;
  imapHost: string;
  imapPort: number;
  username: string;
  createdAt: string;
  messageCount: number;
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
  ai: { ollamaUrl: string; model: string | null };
}

export interface Address {
  name: string | null;
  address: string | null;
}
