import { request } from "./client";

/**
 * Outgoing queue manager and blocked/allowed addresses — contract for
 * `src/api/queue.rs` (views built in `src/mailqueue.rs` and `src/ipaccess.rs`).
 * Envelope and status only: message contents never reach the console.
 */

// ── Queue (GET /queue) ──────────────────────────────────────────────────────

export type QueueRecipientStatus = "waiting" | "retrying" | "failed" | "delivered";
export type QueueProblemKind = "auth" | "tls" | "network" | "rejected" | "address" | "other";

export interface QueueRecipient {
  address: string;
  status: QueueRecipientStatus;
  /** null when nothing has gone wrong (delivered, or not tried yet). */
  kind: QueueProblemKind | null;
  /** The last problem as a sentence. */
  problem: string | null;
  remedy: string | null;
  /** The other server's own words (or the engine's error), verbatim. */
  response: string | null;
  host: string | null;
  attempts: number;
  nextRetry: string | null;
  expiresAt: string | null;
  expiresAfterAttempts: number | null;
}

export interface QueuedMessage {
  id: string;
  /** Envelope sender; "" for bounce notices. */
  sender: string;
  size: number | null;
  createdAt: string | null;
  nextRetry: string | null;
  /** Earliest time the engine gives up on a recipient still pending. */
  expiresAt: string | null;
  recipients: QueueRecipient[];
}

export interface QueueResponse {
  paused: boolean;
  pausedAt: string | null;
  relayed: boolean;
  messages: QueuedMessage[];
}

// ── Login abuse (GET /security/access) ──────────────────────────────────────

export interface BlockedAddress {
  id: string;
  address: string;
  /** authFailure | rcptToFailure | loitering | portScanning | manual | other */
  reason: string;
  reasonText: string;
  createdAt: string | null;
  /** null: until someone unblocks it. */
  expiresAt: string | null;
}

export interface AllowedAddress {
  id: string;
  /** An IP address or CIDR network. */
  address: string;
  reason: string | null;
  createdAt: string | null;
}

export interface BanPolicy {
  enabled: boolean;
  failures: number | null;
  windowMinutes: number | null;
  /** null: bans never expire. */
  banMinutes: number | null;
  recommended: boolean;
}

export interface AccessResponse {
  blocked: BlockedAddress[];
  allowed: AllowedAddress[];
  policy: BanPolicy | null;
}

const id = (v: string) => encodeURIComponent(v);

export const queueApi = {
  list: () => request<QueueResponse>("GET", "/queue"),
  /** One message, or every queued message when `messageId` is omitted. */
  retry: (messageId?: string) => request<{ retried: number }>("POST", "/queue/retry", messageId ? { id: messageId } : {}),
  cancel: (messageId: string) => request<void>("DELETE", `/queue/${id(messageId)}`),
  pause: () => request<QueueResponse>("POST", "/queue/pause"),
  resume: () => request<QueueResponse>("POST", "/queue/resume"),
};

export const accessApi = {
  get: () => request<AccessResponse>("GET", "/security/access"),
  unblock: (blockedId: string) => request<void>("DELETE", `/security/blocked/${id(blockedId)}`),
  allow: (address: string, note?: string) => request<AllowedAddress>("POST", "/security/allowed", { address, note: note || undefined }),
  removeAllowed: (allowedId: string) => request<void>("DELETE", `/security/allowed/${id(allowedId)}`),
};
