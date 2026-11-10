/**
 * Mock of "Move your mail here" (/portal/migrate*) for ./mock.ts. The copy
 * advances with the clock (about 25 seconds end to end), so the progress view
 * can be watched. Password "wrong" fails the sign-in; a private host is refused.
 */
import type { MigrateFolder, MigrationJob } from "./types";

export interface MockMigration {
  job: MigrationJob;
  /** Messages per chosen folder, in order. */
  folders: { name: string; messages: number; bytesPer: number }[];
  startedMs: number;
  cancelledMs: number | null;
  /** Copied before this run (a re-run skips them). */
  alreadyHere: number;
}

type Fail = (status: number, code: string, message: string, field?: string) => never;
type Body = Record<string, unknown>;

const RUN_MS = 25_000;

function f(id: string, messages: number, destination: string, target: MigrateFolder["target"], note: string | null = null): MigrateFolder {
  return { id, name: id, messages, target, destination, selected: note === null, note };
}
const role = (r: "inbox" | "sent" | "drafts" | "trash" | "junk" | "archive") => ({ kind: "role" as const, role: r });
const path = (...parts: string[]) => ({ kind: "path" as const, underInbox: false, parts });

function foldersFor(host: string): MigrateFolder[] {
  if (host.includes("gmail")) {
    return [
      f("INBOX", 1843, "Inbox", role("inbox")),
      f("Receipts", 212, "Receipts", path("Receipts")),
      f("Travel/2025", 38, "Travel / 2025", path("Travel", "2025")),
      f("[Gmail]/All Mail", 6120, "All Mail", path("All Mail"), "Every message again — they're already in your other folders."),
      f("[Gmail]/Drafts", 4, "Drafts", role("drafts")),
      f("[Gmail]/Important", 930, "Important", path("Important"), "Gmail's Important view — the messages are in your other folders."),
      f("[Gmail]/Sent Mail", 1290, "Sent", role("sent")),
      f("[Gmail]/Spam", 61, "Junk", role("junk")),
      f("[Gmail]/Starred", 47, "Starred", path("Starred"), "Starred messages — they're copied with their star from their own folders."),
      f("[Gmail]/Trash", 120, "Trash", role("trash")),
    ];
  }
  if (host.includes("office365") || host.includes("outlook")) {
    return [
      f("Inbox", 2410, "Inbox", role("inbox")),
      f("Archive", 5120, "Archive", role("archive")),
      f("Deleted Items", 88, "Trash", role("trash")),
      f("Drafts", 2, "Drafts", role("drafts")),
      f("Junk Email", 17, "Junk", role("junk")),
      f("Sent Items", 1675, "Sent", role("sent")),
      f("Clients/Acme", 143, "Clients / Acme", path("Clients", "Acme")),
    ];
  }
  return [
    f("INBOX", 640, "Inbox", role("inbox")),
    f("Sent", 210, "Sent", role("sent")),
    f("Drafts", 1, "Drafts", role("drafts")),
    f("Trash", 33, "Trash", role("trash")),
    f("Projects", 57, "Projects", path("Projects")),
  ];
}

function checkSource(body: Body, fail: Fail): { host: string; username: string } {
  const host = (typeof body.host === "string" ? body.host : "").trim().toLowerCase();
  const username = (typeof body.username === "string" ? body.username : "").trim();
  const password = typeof body.password === "string" ? body.password : "";
  if (body.port !== 993 && body.port !== 143)
    fail(400, "invalid_request", "Use port 993 (SSL/TLS) or 143 (STARTTLS). Mail is only fetched over an encrypted connection.", "port");
  if (!/^[a-z0-9-]+(\.[a-z0-9-]+)+$/.test(host)) fail(400, "invalid_request", "Enter the old server's name, like imap.example.com.", "host");
  if (!username) fail(400, "invalid_request", "Enter the username for the old mailbox (usually the full address).", "username");
  if (!password) fail(400, "invalid_request", "Enter the password or app password.", "password");
  if (/^(127\.|10\.|192\.168\.)/.test(host) || host.endsWith(".local") || host.endsWith(".internal"))
    fail(422, "source_failed", `${host} points to a private or local address. Only mail servers on the public internet can be imported from.`);
  if (password === "wrong")
    fail(
      422,
      "source_failed",
      "The old server refused the username or password ([AUTHENTICATIONFAILED] Invalid credentials). Gmail, Microsoft 365, iCloud and Yahoo need an app password here, not your normal password.",
    );
  return { host, username };
}

/** Advance a job to where the clock says it is. */
function tick(m: MockMigration): MigrationJob {
  const job = m.job;
  if (job.state !== "running") return job;
  const until = m.cancelledMs ?? Date.now();
  const frac = Math.min(1, (until - m.startedMs) / RUN_MS);
  const total = m.folders.reduce((n, x) => n + x.messages, 0);
  let todo = Math.floor(frac * total);
  let bytes = 0;
  let done = 0;
  let current: string | null = null;
  for (const x of m.folders) {
    const here = Math.min(todo, x.messages);
    todo -= here;
    bytes += here * x.bytesPer;
    if (here === x.messages) done++;
    else if (current === null) current = x.name;
  }
  const handled = Math.floor(frac * total);
  const skipped = Math.min(handled, m.alreadyHere);
  const failed = handled > 40 ? 1 : 0;
  job.messagesSkipped = skipped;
  job.messagesFailed = failed;
  job.messagesCopied = Math.max(0, handled - skipped - failed);
  job.bytesCopied = Math.round(bytes * (job.messagesCopied / Math.max(1, handled)));
  job.foldersDone = done;
  job.currentFolder = current;
  if (failed && job.errors.length === 0)
    job.errors.push({ folder: m.folders[0]?.name ?? null, message: "A 61.2 MB message is over the 47.7 MB limit and was left on the old server." });
  if (m.cancelledMs !== null || frac >= 1) {
    job.state = m.cancelledMs !== null ? "cancelled" : "done";
    job.finishedAt = new Date(until).toISOString();
    job.currentFolder = null;
    const s = `Copied ${job.messagesCopied.toLocaleString()} message${job.messagesCopied === 1 ? "" : "s"} (${(job.bytesCopied / 1024 ** 2).toFixed(1)} MB)${
      job.messagesSkipped ? `, ${job.messagesSkipped} already here` : ""
    }${job.messagesFailed ? `, ${job.messagesFailed} could not be copied` : ""}.`;
    job.summary = job.state === "cancelled" ? `Stopped. ${s}` : s;
  }
  return job;
}

export function migrateRoute(
  migrations: Record<string, MockMigration>,
  address: string,
  method: string,
  path: string,
  body: Body,
  fail: Fail,
): [number, unknown?] {
  const current = migrations[address];
  if (method === "GET" && path === "/portal/migrate") return [200, { job: current ? tick(current) : null }];
  if (method === "POST" && path === "/portal/migrate/test") {
    const { host } = checkSource(body, fail);
    return [200, { folders: foldersFor(host) }];
  }
  if (method === "POST" && path === "/portal/migrate") {
    if (current && tick(current).state === "running") fail(409, "conflict", "An import into this mailbox is already running.");
    const { host, username } = checkSource(body, fail);
    const all = foldersFor(host);
    const ids = Array.isArray(body.folders) ? (body.folders as unknown[]).filter((x): x is string => typeof x === "string") : null;
    if (ids && ids.length === 0) fail(400, "invalid_request", "Choose at least one folder.", "folders");
    const chosen = all.filter((x) => (ids ? ids.includes(x.id) : x.selected));
    const sameSource = current?.job.source === `${username} on ${host}`;
    const job: MigrationJob = {
      state: "running",
      dryRun: false,
      source: `${username} on ${host}`,
      startedAt: new Date().toISOString(),
      finishedAt: null,
      currentFolder: chosen[0]?.name ?? null,
      foldersDone: 0,
      foldersTotal: chosen.length,
      messagesTotal: chosen.reduce((n, x) => n + x.messages, 0),
      messagesCopied: 0,
      messagesSkipped: 0,
      messagesFailed: 0,
      bytesCopied: 0,
      errors: [],
      summary: null,
    };
    migrations[address] = {
      job,
      folders: chosen.map((x) => ({ name: x.name, messages: x.messages, bytesPer: 48_000 })),
      startedMs: Date.now(),
      cancelledMs: null,
      alreadyHere: sameSource ? current!.job.messagesCopied + current!.job.messagesSkipped : 0,
    };
    return [202, { job }];
  }
  if (method === "POST" && path === "/portal/migrate/cancel") {
    if (!current || tick(current).state !== "running") fail(409, "conflict", "No import is running.");
    current!.cancelledMs = Date.now();
    tick(current!);
    return [204];
  }
  return fail(404, "not_found", "Unknown portal endpoint.");
}
