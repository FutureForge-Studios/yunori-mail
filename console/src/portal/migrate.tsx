import { useEffect, useMemo, useState } from "react";
import { ArrowLeft, ArrowRight, CheckCircle2, CircleAlert, Loader2 } from "lucide-react";
import { Button } from "@/design/vendor/ui/button";
import { Input } from "@/design/vendor/ui/input";
import { Skeleton } from "@/design/vendor/admin/ui";
import { ApiRequestError, errorMessage, portalApi } from "@/api/client";
import type { MigrateFolder, MigrateSource, MigrationJob } from "@/api/types";
import { FormError } from "@/components/auth-frame";
import { ConfirmAction } from "@/components/confirm";
import { FieldError, FilterPills, LoadError } from "@/components/kit";
import { PasswordInput } from "@/components/password-input";
import { fmtBytes, fmtDateTime, fmtNumber } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";

type ProviderKey = "gmail" | "m365" | "icloud" | "fastmail" | "yahoo" | "other";

interface Provider {
  label: string;
  host: string;
  port: number;
  /** One line on getting the password this needs. */
  password: string;
}

/** Well-known IMAP servers. OAuth sign-in isn't supported, so each needs an app password. */
export const PROVIDERS: Record<ProviderKey, Provider> = {
  gmail: {
    label: "Gmail",
    host: "imap.gmail.com",
    port: 993,
    password: "Use an app password: turn on 2-Step Verification, then create one at myaccount.google.com/apppasswords.",
  },
  m365: {
    label: "Microsoft 365",
    host: "outlook.office365.com",
    port: 993,
    password:
      "Use an app password from mysignins.microsoft.com → Security info (your organisation must allow IMAP and app passwords).",
  },
  icloud: {
    label: "iCloud",
    host: "imap.mail.me.com",
    port: 993,
    password: "Use an app-specific password from account.apple.com → Sign-In and Security.",
  },
  fastmail: {
    label: "Fastmail",
    host: "imap.fastmail.com",
    port: 993,
    password: "Use an app password with IMAP access, from Settings → Privacy & Security.",
  },
  yahoo: {
    label: "Yahoo",
    host: "imap.mail.yahoo.com",
    port: 993,
    password: "Use an app password from Account Info → Account security → Generate app password.",
  },
  other: {
    label: "Other",
    host: "",
    port: 993,
    password: "Your mailbox password, or an app password if your provider uses them.",
  },
};

const INPUT_ID: Record<string, string> = { host: "mv-host", port: "mv-port", username: "mv-user", password: "mv-password", folders: "mv-folders" };

const RUNNING_POLL_MS = 2000;

/**
 * "Move your mail here": copy an old mailbox (Gmail, Microsoft 365, any
 * IMAP server) into this one. Test the connection, choose folders, then the
 * server copies in the background — the page can be closed.
 */
export function MoveMail({ address }: { address: string }) {
  const { data, error, reload, setData } = useApi("portal:migrate", portalApi.migrateStatus);
  const [composing, setComposing] = useState(false);
  const job = data?.job ?? null;
  const running = job?.state === "running";

  // Poll while it runs.
  useEffect(() => {
    if (!running) return;
    const t = window.setInterval(() => {
      portalApi
        .migrateStatus()
        .then((r) => {
          setData(r);
          if (r.job && r.job.state !== "running") {
            if (r.job.state === "done") toast.success(r.job.summary ?? "Your mail is here.");
          }
        })
        .catch(() => undefined);
    }, RUNNING_POLL_MS);
    return () => window.clearInterval(t);
  }, [running, setData]);

  if (error && !data) return <LoadError error={error} onRetry={() => void reload()} what="Moving your mail" />;
  if (!data)
    return (
      <div aria-busy="true" className="rounded-xl border border-border/60 bg-card/40 p-5">
        <Skeleton className="h-5 w-56" />
        <Skeleton className="mt-6 h-9 w-full" />
      </div>
    );
  if (job && running) return <JobCard job={job} />;
  if (job && !composing) return <JobCard job={job} onAgain={() => setComposing(true)} />;
  return (
    <ImportForm
      address={address}
      onCancel={job ? () => setComposing(false) : undefined}
      onStarted={(j) => {
        setComposing(false);
        setData({ job: j });
      }}
    />
  );
}

// ── form: where from, test, choose folders ───────────────────────────────────

function ImportForm({ address, onStarted, onCancel }: { address: string; onStarted: (j: MigrationJob) => void; onCancel?: () => void }) {
  const [provider, setProvider] = useState<ProviderKey>("gmail");
  const [host, setHost] = useState("");
  const [port, setPort] = useState(993);
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [folders, setFolders] = useState<MigrateFolder[] | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<string | null>(null);
  const p = PROVIDERS[provider];

  useEffect(() => {
    if (!focus) return;
    document.getElementById(INPUT_ID[focus] ?? "")?.focus();
    setFocus(null);
  }, [focus]);

  function source(): MigrateSource {
    return provider === "other"
      ? { host: host.trim(), port, username: username.trim(), password }
      : { host: p.host, port: p.port, username: username.trim(), password };
  }

  function showError(err: unknown) {
    if (err instanceof ApiRequestError && err.field && INPUT_ID[err.field]) {
      setErrors({ [err.field]: err.message });
      setFocus(err.field);
    } else setErrors({ form: errorMessage(err) });
  }

  async function test(e: React.FormEvent) {
    e.preventDefault();
    const errs: Record<string, string> = {};
    if (provider === "other" && !host.trim()) errs.host = "Enter the old server's name, like imap.example.com.";
    if (!username.trim()) errs.username = "Enter the username for the old mailbox (usually its address).";
    if (!password) errs.password = "Enter the password or app password.";
    setErrors(errs);
    const first = ["host", "username", "password"].find((k) => errs[k]);
    if (first) {
      setFocus(first);
      return;
    }
    setBusy(true);
    try {
      const r = await portalApi.migrateTest(source());
      setFolders(r.folders);
      setChosen(new Set(r.folders.filter((f) => f.selected).map((f) => f.id)));
    } catch (err) {
      showError(err);
    } finally {
      setBusy(false);
    }
  }

  async function start() {
    if (!folders) return;
    if (chosen.size === 0) {
      setErrors({ folders: "Choose at least one folder." });
      return;
    }
    setBusy(true);
    setErrors({});
    try {
      const r = await portalApi.migrateStart({ ...source(), folders: folders.filter((f) => chosen.has(f.id)).map((f) => f.id) });
      setPassword("");
      if (r.job) onStarted(r.job);
    } catch (err) {
      showError(err);
    } finally {
      setBusy(false);
    }
  }

  if (folders) {
    return (
      <FolderPicker
        folders={folders}
        chosen={chosen}
        onChosen={(s) => {
          setChosen(s);
          if (errors.folders) setErrors({});
        }}
        from={`${username.trim()} · ${provider === "other" ? host.trim() : p.label}`}
        address={address}
        busy={busy}
        errors={errors}
        onBack={() => {
          setFolders(null);
          setErrors({});
        }}
        onStart={() => void start()}
      />
    );
  }

  return (
    <form onSubmit={test} noValidate className="rounded-xl border border-border/60 bg-card/40">
      <div className="border-b border-border/50 px-5 py-3">
        <FilterPills
          label="Where your mail is now"
          value={provider}
          onChange={(v) => {
            setProvider(v);
            setErrors({});
          }}
          options={(Object.keys(PROVIDERS) as ProviderKey[]).map((k) => ({ value: k, label: PROVIDERS[k].label }))}
        />
      </div>
      <div className="space-y-4 p-5">
        {provider === "other" && (
          <div className="grid gap-4 sm:grid-cols-[1fr_9rem]">
            <div className="space-y-1.5">
              <FieldLabel htmlFor="mv-host">IMAP server</FieldLabel>
              <Input
                id="mv-host"
                value={host}
                onChange={(e) => setHost(e.target.value)}
                placeholder="imap.example.com"
                autoCapitalize="none"
                spellCheck={false}
                aria-invalid={!!errors.host}
                aria-describedby={errors.host ? "mv-host-err" : undefined}
              />
              <FieldError id="mv-host-err">{errors.host}</FieldError>
            </div>
            <div className="space-y-1.5">
              <FieldLabel htmlFor="mv-port">Security</FieldLabel>
              <select
                id="mv-port"
                value={port}
                onChange={(e) => setPort(Number(e.target.value))}
                className="h-9 w-full rounded-md border border-input bg-transparent px-2 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                aria-invalid={!!errors.port}
                aria-describedby={errors.port ? "mv-port-err" : undefined}
              >
                <option value={993}>993 · SSL/TLS</option>
                <option value={143}>143 · STARTTLS</option>
              </select>
              <FieldError id="mv-port-err">{errors.port}</FieldError>
            </div>
          </div>
        )}
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-1.5">
            <FieldLabel htmlFor="mv-user">{provider === "other" ? "Username" : `${p.label} address`}</FieldLabel>
            <Input
              id="mv-user"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              type="email"
              inputMode="email"
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              placeholder={provider === "gmail" ? "you@gmail.com" : "you@example.com"}
              aria-invalid={!!errors.username}
              aria-describedby={errors.username ? "mv-user-err" : undefined}
            />
            <FieldError id="mv-user-err">{errors.username}</FieldError>
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor="mv-password">{provider === "other" ? "Password" : "App password"}</FieldLabel>
            <PasswordInput
              id="mv-password"
              autoComplete="off"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              aria-invalid={!!errors.password}
              aria-describedby={errors.password ? "mv-password-err" : "mv-password-hint"}
            />
            {errors.password ? (
              <FieldError id="mv-password-err">{errors.password}</FieldError>
            ) : (
              <p id="mv-password-hint" className="text-[11px] leading-relaxed text-muted-foreground">
                {p.password}
              </p>
            )}
          </div>
        </div>
        <p className="text-[11px] leading-relaxed text-muted-foreground">
          Only read, never changed: your old mailbox stays exactly as it is. The password is used for this copy only and never stored.
        </p>
      </div>
      <div className="flex flex-col gap-3 border-t border-border/50 px-5 py-4 sm:flex-row sm:items-center sm:justify-end">
        {errors.form && (
          <div className="min-w-0 flex-1">
            <FormError>{errors.form}</FormError>
          </div>
        )}
        {onCancel && (
          <Button type="button" variant="ghost" size="sm" onClick={onCancel} className="self-end sm:self-auto">
            Cancel
          </Button>
        )}
        <Button type="submit" size="sm" disabled={busy} className="shrink-0 self-end sm:self-auto">
          {busy ? (
            <>
              <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />
              Connecting…
            </>
          ) : (
            <>
              Connect and list folders
              <ArrowRight className="h-3.5 w-3.5" />
            </>
          )}
        </Button>
      </div>
    </form>
  );
}

function FolderPicker({
  folders,
  chosen,
  onChosen,
  from,
  address,
  busy,
  errors,
  onBack,
  onStart,
}: {
  folders: MigrateFolder[];
  chosen: Set<string>;
  onChosen: (s: Set<string>) => void;
  from: string;
  address: string;
  busy: boolean;
  errors: Record<string, string>;
  onBack: () => void;
  onStart: () => void;
}) {
  const total = useMemo(() => folders.filter((f) => chosen.has(f.id)).reduce((n, f) => n + f.messages, 0), [folders, chosen]);
  const toggle = (id: string) => {
    const next = new Set(chosen);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    onChosen(next);
  };
  const allDefault = () => onChosen(new Set(folders.filter((f) => f.selected).map((f) => f.id)));
  return (
    <div className="rounded-xl border border-border/60 bg-card/40">
      <div className="flex items-start gap-3 border-b border-border/50 px-5 py-3">
        <CheckCircle2 aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-emerald-700 dark:text-emerald-400" />
        <p className="min-w-0 text-xs leading-relaxed text-muted-foreground">
          Connected to <span className="break-all text-foreground/90">{from}</span>. Choose what to copy into{" "}
          <span className="break-all text-foreground/90">{address}</span>.
        </p>
      </div>
      <fieldset id="mv-folders" tabIndex={-1} aria-describedby={errors.folders ? "mv-folders-err" : undefined} className="focus:outline-none">
        <legend className="sr-only">Folders to copy</legend>
        <ul className="max-h-[26rem] divide-y divide-border/50 overflow-y-auto">
          {folders.map((f) => (
            <li key={f.id}>
              <label className="flex cursor-pointer items-start gap-3 px-5 py-2.5 hover:bg-accent/40">
                <input
                  type="checkbox"
                  checked={chosen.has(f.id)}
                  onChange={() => toggle(f.id)}
                  className="mt-1 h-3.5 w-3.5 shrink-0 accent-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                />
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline gap-2">
                    <span className="min-w-0 truncate text-sm text-foreground/90">{f.name}</span>
                    <span className="ml-auto shrink-0 text-xs tabular-nums text-muted-foreground">{fmtNumber(f.messages)}</span>
                  </span>
                  <span className="block text-[11px] text-muted-foreground">
                    {f.note && !chosen.has(f.id) ? f.note : <>→ {f.destination}</>}
                  </span>
                </span>
              </label>
            </li>
          ))}
        </ul>
      </fieldset>
      <div className="flex flex-col gap-3 border-t border-border/50 px-5 py-4 sm:flex-row sm:items-center">
        <div className="min-w-0 flex-1 text-xs text-muted-foreground">
          <p className="tabular-nums text-foreground/85">
            {chosen.size} folder{chosen.size === 1 ? "" : "s"} · {fmtNumber(total)} message{total === 1 ? "" : "s"}
          </p>
          <p className="mt-0.5">
            <button type="button" onClick={() => onChosen(new Set(folders.map((f) => f.id)))} className="underline-offset-2 hover:text-foreground hover:underline">
              All
            </button>
            {" · "}
            <button type="button" onClick={allDefault} className="underline-offset-2 hover:text-foreground hover:underline">
              Suggested
            </button>
            {" · "}
            <button type="button" onClick={() => onChosen(new Set())} className="underline-offset-2 hover:text-foreground hover:underline">
              None
            </button>
          </p>
          <FieldError id="mv-folders-err">{errors.folders}</FieldError>
          {errors.form && <FormError>{errors.form}</FormError>}
        </div>
        <div className="flex shrink-0 items-center justify-end gap-2">
          <Button type="button" variant="ghost" size="sm" onClick={onBack} disabled={busy}>
            <ArrowLeft className="h-3.5 w-3.5" />
            Back
          </Button>
          <Button type="button" variant="torii" size="sm" onClick={onStart} disabled={busy || chosen.size === 0}>
            {busy ? "Starting…" : "Start copying"}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ── progress and result ──────────────────────────────────────────────────────

const STATE_TEXT: Record<MigrationJob["state"], string> = {
  running: "Copying your mail",
  done: "Your mail is here",
  failed: "The copy stopped",
  cancelled: "You stopped the copy",
  interrupted: "The copy was interrupted",
};

function JobCard({ job, onAgain }: { job: MigrationJob; onAgain?: () => void }) {
  const [stopping, setStopping] = useState(false);
  const [showErrors, setShowErrors] = useState(false);
  const running = job.state === "running";
  const handled = job.messagesCopied + job.messagesSkipped + job.messagesFailed;
  const value = job.messagesTotal > 0 ? Math.min(100, (handled / job.messagesTotal) * 100) : running ? 0 : 100;
  const ok = job.state === "done" && job.messagesFailed === 0;

  return (
    <div className="rounded-xl border border-border/60 bg-card/40">
      <div className="space-y-3 p-5">
        <div className="flex items-start gap-2.5">
          {running ? (
            <Loader2 aria-hidden className="mt-0.5 h-4 w-4 shrink-0 animate-spin text-muted-foreground motion-reduce:animate-none" />
          ) : ok ? (
            <CheckCircle2 aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-emerald-700 dark:text-emerald-400" />
          ) : (
            <CircleAlert aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-amber-700 dark:text-amber-400" />
          )}
          <div className="min-w-0">
            <p className="text-sm font-medium">
              {STATE_TEXT[job.state]}
            </p>
            <p className="mt-0.5 break-all text-xs text-muted-foreground">From {job.source}</p>
          </div>
        </div>
        <div
          role="progressbar"
          aria-label="Messages copied"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(value)}
          aria-valuetext={`${fmtNumber(handled)} of ${fmtNumber(job.messagesTotal)} messages`}
          className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
        >
          <div
            className={`h-full rounded-full transition-[width] duration-500 motion-reduce:transition-none ${ok ? "bg-emerald-600 dark:bg-emerald-400" : "bg-foreground/60"}`}
            style={{ width: `${Math.max(value, value > 0 ? 2 : 0)}%` }}
          />
        </div>
        <p role="status" aria-live="polite" className="text-xs tabular-nums leading-relaxed text-foreground/85">
          {running ? (
            <>
              {fmtNumber(handled)} of {fmtNumber(job.messagesTotal)} messages · {fmtBytes(job.bytesCopied)} · folder{" "}
              {Math.min(job.foldersDone + 1, job.foldersTotal)} of {job.foldersTotal}
              {job.currentFolder && <span className="text-muted-foreground"> · {job.currentFolder}</span>}
            </>
          ) : (
            job.summary
          )}
        </p>
        {running && (
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            You can close this page — the server keeps copying. Running it again later copies only what's new; nothing is copied twice.
          </p>
        )}
        {!running && job.finishedAt && <p className="text-[11px] text-muted-foreground">Finished {fmtDateTime(job.finishedAt)}</p>}
        {job.errors.length > 0 && (
          <div>
            <button
              type="button"
              onClick={() => setShowErrors((s) => !s)}
              aria-expanded={showErrors}
              className="text-xs text-amber-800 underline-offset-2 hover:underline dark:text-amber-300"
            >
              {job.messagesFailed > 0
                ? `${fmtNumber(job.messagesFailed)} message${job.messagesFailed === 1 ? "" : "s"} couldn't be copied`
                : `${job.errors.length} problem${job.errors.length === 1 ? "" : "s"}`}{" "}
              — {showErrors ? "hide" : "show"} details
            </button>
            {showErrors && (
              <ul className="mt-2 space-y-1 text-[11px] leading-relaxed text-muted-foreground">
                {job.errors.map((e, i) => (
                  <li key={i}>
                    {e.folder && <span className="text-foreground/80">{e.folder}: </span>}
                    {e.message}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
      </div>
      <div className="flex justify-end gap-2 border-t border-border/50 px-5 py-3">
        {running ? (
          <Button variant="ghost" size="sm" onClick={() => setStopping(true)} className="text-muted-foreground hover:text-destructive">
            Stop
          </Button>
        ) : (
          onAgain && (
            <Button variant="outline" size="sm" onClick={onAgain}>
              {job.state === "done" ? "Copy again or from another account" : "Start again"}
            </Button>
          )
        )}
      </div>
      <ConfirmAction
        open={stopping}
        onOpenChange={setStopping}
        title="Stop copying?"
        description="What's copied so far stays here. Start again any time to copy the rest — nothing is copied twice."
        confirmLabel="Stop"
        destructive
        onConfirm={async () => {
          await portalApi.migrateCancel();
          toast.success("Stopping after the current message.");
        }}
      />
    </div>
  );
}

function FieldLabel({ htmlFor, children }: { htmlFor: string; children: React.ReactNode }) {
  return (
    <label htmlFor={htmlFor} className="block text-xs font-medium text-foreground/80">
      {children}
    </label>
  );
}
