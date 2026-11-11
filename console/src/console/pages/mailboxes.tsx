import { useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { CheckCircle2, Download, ExternalLink, Gauge, HardDrive, Inbox, KeyRound, Link2, MailPlus, MoreHorizontal, Pencil, Plus, Trash2, Upload } from "lucide-react";
import { Button } from "@/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/ui/overlay";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/design/vendor/ui/dropdown-menu";
import { AddressInput, Field, Input, RadioCard, Select } from "@/ui/form";
import { Avatar, Badge } from "@/ui/badge";
import { FormError } from "@/ui/feedback";
import { SearchInput } from "@/ui/segmented";
import { TableToolbar } from "@/ui/data-table";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { CreateMailboxResponse, Domain, Mailbox } from "@/api/types";
import { ConfirmAction } from "@/components/confirm";
import { DataTable, FieldError, LoadError, Page, usePageTitle, type Column } from "@/components/kit";
import { Meter, StatCard, usageTone } from "@/ui/data-viz";
import { PasswordInput, StrengthHint } from "@/components/password-input";
import { SecretReveal } from "@/components/secret-reveal";
import { fmtBytes, fmtDate, fmtNumber, openInYunoriUrl, pct, relativeDays, timeAgo } from "@/lib/format";
import { saveBlob } from "@/lib/download";
import { MIN_PASSWORD } from "@/lib/password";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";
import { ImportMailboxesDialog } from "../mailbox-import";
import { useSession } from "../session";

const QUOTAS: { mb: number | null; label: string }[] = [
  { mb: 1024, label: "1 GB" },
  { mb: 2048, label: "2 GB" },
  { mb: 5120, label: "5 GB" },
  { mb: 10240, label: "10 GB" },
  { mb: 25600, label: "25 GB" },
  { mb: 51200, label: "50 GB" },
  { mb: null, label: "No limit" },
];

type Reveal =
  | { kind: "created" | "reset"; address: string; password: string }
  | { kind: "invite"; address: string; name: string | null; url: string; expiresAt: string; isNew: boolean };

/** Mailboxes: create (invite link, or a password shown once), import/export CSV, reset, edit, delete. */
export default function MailboxesPage() {
  usePageTitle("Mailboxes");
  const { session } = useSession();
  const [params, setParams] = useSearchParams();
  const { data, error, loading, reload } = useApi("mailboxes", async () => {
    const [mailboxes, domains] = await Promise.all([api.mailboxes(), api.domains()]);
    return { mailboxes, domains };
  });
  const [q, setQ] = useState(params.get("q") ?? "");
  const domainFilter = params.get("domain") ?? "all";
  const [createOpen, setCreateOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [reveal, setReveal] = useState<Reveal | null>(null);
  const [editing, setEditing] = useState<Mailbox | null>(null);
  const [resetting, setResetting] = useState<Mailbox | null>(null);
  const [deleting, setDeleting] = useState<Mailbox | null>(null);

  async function invite(m: Mailbox) {
    try {
      const r = await api.inviteMailbox(m.id);
      setReveal({ kind: "invite", address: m.address, name: m.displayName, url: r.url, expiresAt: r.expiresAt, isNew: false });
      void reload();
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }

  async function exportCsv() {
    setExporting(true);
    try {
      const { blob, filename } = await api.exportMailboxes();
      saveBlob(blob, filename);
    } catch (err) {
      toast.error(errorMessage(err, "The export didn't download. Try again."));
    } finally {
      setExporting(false);
    }
  }

  useEffect(() => {
    if (params.get("new") === "1") {
      setCreateOpen(true);
      setParams(
        (p) => {
          const n = new URLSearchParams(p);
          n.delete("new");
          return n;
        },
        { replace: true },
      );
    }
  }, [params, setParams]);

  const setDomainFilter = (v: string) =>
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        if (v === "all") n.delete("domain");
        else n.set("domain", v);
        return n;
      },
      { replace: true },
    );

  const rows = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (data?.mailboxes ?? [])
      .filter((m) => domainFilter === "all" || m.domainId === domainFilter)
      .filter((m) => !needle || m.address.includes(needle) || (m.displayName ?? "").toLowerCase().includes(needle) || m.aliases.some((a) => a.includes(needle)))
      .sort((a, b) => a.address.localeCompare(b.address));
  }, [data, q, domainFilter]);

  const yunori = (address: string) => openInYunoriUrl(session.server.clientOrigin, address);

  const columns: Column<Mailbox>[] = [
    {
      key: "address",
      header: "Mailbox",
      cell: (m) => (
        <div className="flex min-w-0 items-center gap-3">
          <Avatar name={m.displayName ?? m.address} />
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="break-all text-sm font-medium text-foreground">{m.address}</span>
              {m.role === "admin" && <Badge>Admin</Badge>}
              {m.local === "postmaster" && <Badge>System</Badge>}
              {m.inviteExpiresAt && (
                <span title={`The invite link works until ${fmtDate(m.inviteExpiresAt)}`}>
                  <Badge tone="warn">Invite pending</Badge>
                </span>
              )}
            </div>
            <p className="mt-0.5 text-[13px] text-muted-foreground">{m.displayName ?? "No display name"}</p>
          </div>
        </div>
      ),
    },
    {
      key: "storage",
      header: "Storage",
      className: "w-52",
      cell: (m) => {
        const p = pct(m.usedBytes, m.quotaBytes);
        const text = m.quotaBytes ? `${fmtBytes(m.usedBytes ?? 0)} of ${fmtBytes(m.quotaBytes)}` : `${fmtBytes(m.usedBytes ?? 0)} · no limit`;
        return (
          <div className="w-full max-w-[11rem]">
            <Meter value={p} label={`Storage for ${m.address}`} valueText={text} tone={usageTone(p)} />
            <p className="mt-1.5 text-xs tabular-nums text-muted-foreground">
              {text}
              {p !== null && p >= 90 && <span className="font-medium text-warning-ink"> · nearly full</span>}
            </p>
          </div>
        );
      },
    },
    {
      key: "aliases",
      header: "Aliases",
      hideBelowMd: true,
      cell: (m) =>
        m.aliases.length === 0 ? (
          <span className="text-[13px] text-muted-foreground">—</span>
        ) : (
          <span className="text-[13px] text-foreground/80" title={m.aliases.join(", ")}>
            {m.aliases[0]}
            {m.aliases.length > 1 && <span className="text-muted-foreground"> +{m.aliases.length - 1}</span>}
          </span>
        ),
    },
    {
      key: "created",
      header: "Created",
      hideBelowMd: true,
      cell: (m) => <span className="text-[13px] text-muted-foreground">{timeAgo(m.createdAt)}</span>,
    },
    {
      key: "actions",
      header: <span className="sr-only">Actions</span>,
      className: "w-12 text-right",
      cell: (m) => (
        <DropdownMenu>
          <DropdownMenuTrigger
            aria-label={`Actions for ${m.address}`}
            className="grid h-8 w-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 data-[state=open]:bg-accent"
          >
            <MoreHorizontal className="h-4 w-4" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-56">
            {m.role !== "admin" && m.local !== "postmaster" && (
              <DropdownMenuItem onSelect={() => void invite(m)} className="text-xs">
                <Link2 className="h-3.5 w-3.5" aria-hidden />
                {m.inviteExpiresAt ? "New invite link" : "Send an invite link"}
              </DropdownMenuItem>
            )}
            <DropdownMenuItem onSelect={() => setResetting(m)} className="text-xs">
              <KeyRound className="h-3.5 w-3.5" aria-hidden />
              Reset password
            </DropdownMenuItem>
            <DropdownMenuItem onSelect={() => setEditing(m)} className="text-xs">
              <Pencil className="h-3.5 w-3.5" aria-hidden />
              Edit name & storage
            </DropdownMenuItem>
            <DropdownMenuItem asChild className="text-xs">
              <a href={yunori(m.address)} target="_blank" rel="noopener noreferrer">
                <ExternalLink className="h-3.5 w-3.5" aria-hidden />
                Open in Yunori
              </a>
            </DropdownMenuItem>
            {m.local !== "postmaster" && (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => setDeleting(m)} className="text-xs text-destructive focus:text-destructive">
                  <Trash2 className="h-3.5 w-3.5" aria-hidden />
                  Delete mailbox
                </DropdownMenuItem>
              </>
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ];

  const domains = data?.domains ?? [];

  return (
    <Page
      title="Mailboxes"
      blurb={
        data
          ? `${fmtNumber(data.mailboxes.length)} mailbox${data.mailboxes.length === 1 ? "" : "es"} across ${domains.length} domain${domains.length === 1 ? "" : "s"}. The console never sees what's inside them.`
          : "The console never sees what's inside a mailbox."
      }
      actions={
        <>
          <Button size="sm" variant="outline" onClick={() => setImportOpen(true)} disabled={!data || domains.length === 0}>
            <Upload className="h-3.5 w-3.5" />
            Import
          </Button>
          <Button size="sm" variant="outline" onClick={() => void exportCsv()} disabled={!data || data.mailboxes.length === 0 || exporting}>
            <Download className="h-3.5 w-3.5" />
            {exporting ? "Exporting…" : "Export"}
          </Button>
          <Button size="sm" onClick={() => setCreateOpen(true)} disabled={!data || domains.length === 0}>
            <Plus className="h-4 w-4" />
            New mailbox
          </Button>
        </>
      }
    >
      {reveal && <RevealPanel reveal={reveal} openUrl={yunori(reveal.address)} onDone={() => setReveal(null)} />}

      {data && data.mailboxes.length > 0 && <Summary mailboxes={data.mailboxes} />}

      <TableToolbar end={data && <span className="text-[13px] tabular-nums text-muted-foreground">{fmtNumber(rows.length)} shown</span>}>
        <SearchInput value={q} onValueChange={setQ} placeholder="Search address, name or alias…" aria-label="Search mailboxes" />
        {domains.length > 1 && (
          <Select aria-label="Filter by domain" value={domainFilter} onChange={(e) => setDomainFilter(e.target.value)} wrapperClassName="sm:w-48">
            <option value="all">All domains</option>
            {domains.map((d) => (
              <option key={d.id} value={d.id}>
                {d.name}
              </option>
            ))}
          </Select>
        )}
      </TableToolbar>

      {error && !data ? (
        <LoadError error={error} onRetry={() => void reload()} what="Mailboxes" />
      ) : (
        <DataTable
          columns={columns}
          rows={rows}
          rowKey={(m) => m.id}
          loading={loading && !data}
          caption="Mailboxes"
          emptyIcon={Inbox}
          emptyTitle={q || domainFilter !== "all" ? "No mailboxes match" : "No mailboxes yet"}
          emptyHint={q || domainFilter !== "all" ? "Try another search or domain." : "Create the first one and send its owner an invite link — they choose their own password."}
        />
      )}

      <CreateMailboxDialog
        open={createOpen}
        onOpenChange={setCreateOpen}
        domains={domains}
        defaultDomainId={domainFilter !== "all" ? domainFilter : undefined}
        onCreated={(r) => {
          if (r.invite)
            setReveal({ kind: "invite", address: r.mailbox.address, name: r.mailbox.displayName, url: r.invite.url, expiresAt: r.invite.expiresAt, isNew: true });
          else setReveal({ kind: "created", address: r.mailbox.address, password: r.password ?? "" });
          void reload();
        }}
      />
      <ImportMailboxesDialog open={importOpen} onOpenChange={setImportOpen} onImported={() => void reload()} />
      {editing && <EditMailboxDialog mailbox={editing} onClose={() => setEditing(null)} onSaved={() => void reload()} />}
      <ConfirmAction
        open={!!resetting}
        onOpenChange={(o) => !o && setResetting(null)}
        title={`Reset the password for ${resetting?.address ?? ""}?`}
        description="The current password stops working right away, and mail apps using it will ask again. App passwords keep working. The new password is shown once, and an unused invite link stops working. To let them choose a password instead, send an invite link."
        confirmLabel="Reset password"
        onConfirm={async () => {
          const m = resetting!;
          const r = await api.resetMailboxPassword(m.id);
          setReveal({ kind: "reset", address: m.address, password: r.password });
        }}
      />
      <ConfirmAction
        open={!!deleting}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`Delete ${deleting?.address ?? ""}?`}
        description="Every message in this mailbox is deleted permanently. Mail sent to the address afterwards is refused."
        confirmLabel="Delete mailbox"
        destructive
        typeToConfirm={deleting?.address}
        onConfirm={async () => {
          const m = deleting!;
          await api.deleteMailbox(m.id);
          toast.success(`${m.address} was deleted.`);
          void reload();
        }}
      >
        {deleting && <Dependents mailbox={deleting} domains={domains} />}
      </ConfirmAction>
    </Page>
  );
}

function Dependents({ mailbox, domains }: { mailbox: Mailbox; domains: Domain[] }) {
  const catchAllFor = domains.filter((d) => d.catchAll === mailbox.address).map((d) => d.name);
  if (mailbox.aliases.length === 0 && catchAllFor.length === 0) return null;
  return (
    <div className="rounded-lg border border-border bg-muted/40 px-3 py-2.5 text-[13px] leading-relaxed">
      <p className="font-medium text-foreground">Also affected</p>
      <ul className="mt-1 list-disc space-y-0.5 pl-4 text-foreground/80">
        {mailbox.aliases.length > 0 && (
          <li>
            {mailbox.aliases.length} alias{mailbox.aliases.length === 1 ? "" : "es"} stop working: {mailbox.aliases.join(", ")}
          </li>
        )}
        {catchAllFor.length > 0 && <li>Catch-all for {catchAllFor.join(", ")} turns off.</li>}
      </ul>
    </div>
  );
}

function RevealPanel({ reveal, openUrl, onDone }: { reveal: Reveal; openUrl: string; onDone: () => void }) {
  const ref = useRef<HTMLHeadingElement>(null);
  useEffect(() => ref.current?.focus(), [reveal]);
  if (reveal.kind === "invite") return <InvitePanel reveal={reveal} headingRef={ref} onDone={onDone} />;
  return (
    <section className="mb-6 animate-rise-in rounded-xl border border-success/30 bg-card p-5 shadow-pop motion-reduce:animate-none" aria-labelledby="reveal-title">
      <h2 id="reveal-title" ref={ref} tabIndex={-1} className="flex items-center gap-2 text-[15px] font-semibold focus:outline-none">
        <CheckCircle2 aria-hidden className="h-4 w-4 shrink-0 text-success-ink" />
        {reveal.kind === "created" ? (
          <>
            <span className="font-mono">{reveal.address}</span> is ready
          </>
        ) : (
          <>
            New password for <span className="font-mono">{reveal.address}</span>
          </>
        )}
      </h2>
      <p className="mt-1 text-[13px] text-muted-foreground">
        {reveal.kind === "created"
          ? "Give these to the person who'll use it, through a channel you trust. They can change the password in their own settings."
          : "Share it through a channel you trust. They can change it afterwards in their own settings."}
      </p>
      <div className="mt-4 grid gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.4fr)]">
        <div className="rounded-xl border border-border bg-muted/40 p-4">
          <p className="text-xs font-medium text-muted-foreground">Address</p>
          <p className="mt-2 break-all font-mono text-[15px]">{reveal.address}</p>
          <p className="mt-2 text-xs text-muted-foreground">Also the username in any mail app.</p>
        </div>
        <SecretReveal label="Password" value={reveal.password} />
      </div>
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <Button asChild variant="outline" size="sm">
          <a href={openUrl} target="_blank" rel="noopener noreferrer">
            <ExternalLink className="h-3.5 w-3.5" />
            Open in Yunori
          </a>
        </Button>
        <Button size="sm" onClick={onDone} className="ml-auto">
          I've saved it
        </Button>
      </div>
    </section>
  );
}

function InvitePanel({
  reveal,
  headingRef,
  onDone,
}: {
  reveal: Extract<Reveal, { kind: "invite" }>;
  headingRef: React.RefObject<HTMLHeadingElement | null>;
  onDone: () => void;
}) {
  const who = reveal.name ?? reveal.address;
  return (
    <section className="mb-6 animate-rise-in rounded-xl border border-success/30 bg-card p-5 shadow-pop motion-reduce:animate-none" aria-labelledby="reveal-title">
      <h2 id="reveal-title" ref={headingRef} tabIndex={-1} className="flex flex-wrap items-center gap-x-2 text-[15px] font-semibold focus:outline-none">
        <MailPlus aria-hidden className="h-4 w-4 shrink-0 text-success-ink" />
        {reveal.isNew ? (
          <>
            <span className="font-mono">{reveal.address}</span> is ready — send {reveal.name ? reveal.name : "its owner"} this link
          </>
        ) : (
          <>
            Invite link for <span className="font-mono">{reveal.address}</span>
          </>
        )}
      </h2>
      <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">
        {who} opens it, chooses a password, and lands on a page that sets up their phone, computer and Yunori. Nobody else ever sees
        the password, you included.
      </p>
      <div className="mt-4">
        <SecretReveal
          label="Invite link"
          value={reveal.url}
          note={
            <>
              Works once, until {fmtDate(reveal.expiresAt)} ({relativeDays(reveal.expiresAt)}). Send it any way you'd send something
              private — a chat message, a text, in person. Making a new link cancels this one.
            </>
          }
        />
      </div>
      <div className="mt-4 flex justify-end">
        <Button size="sm" onClick={onDone}>
          Done
        </Button>
      </div>
    </section>
  );
}

function CreateMailboxDialog({
  open,
  onOpenChange,
  domains,
  defaultDomainId,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  domains: Domain[];
  defaultDomainId?: string;
  onCreated: (r: CreateMailboxResponse) => void;
}) {
  const primary = domains.find((d) => d.isPrimary)?.id ?? domains[0]?.id ?? "";
  const [local, setLocal] = useState("");
  const [domainId, setDomainId] = useState(defaultDomainId ?? primary);
  const [displayName, setDisplayName] = useState("");
  const [quota, setQuota] = useState<string>("5120");
  const [mode, setMode] = useState<"invite" | "generate" | "manual">("invite");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});

  useEffect(() => {
    if (open) setDomainId(defaultDomainId ?? primary);
  }, [open, defaultDomainId, primary]);

  const reset = () => {
    setLocal("");
    setDisplayName("");
    setQuota("5120");
    setMode("invite");
    setPassword("");
    setErrors({});
  };

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const local_ = local.trim().toLowerCase();
    const errs: Record<string, string> = {};
    if (!/^[a-z0-9](?:[a-z0-9._+-]{0,62}[a-z0-9])?$/.test(local_)) errs.local = "Use letters, numbers, dots, dashes or underscores.";
    if (mode === "manual" && password.length < MIN_PASSWORD) errs.password = `Use at least ${MIN_PASSWORD} characters.`;
    if (Object.keys(errs).length) {
      setErrors(errs);
      return;
    }
    setBusy(true);
    setErrors({});
    try {
      const r = await api.createMailbox({
        local: local_,
        domainId,
        displayName: displayName.trim() || undefined,
        quotaMb: quota === "" ? null : Number(quota),
        password: mode === "manual" ? password : undefined,
        invite: mode === "invite" || undefined,
      });
      onOpenChange(false);
      reset();
      onCreated(r);
    } catch (err) {
      if (err instanceof ApiRequestError && err.field) setErrors({ [err.field]: err.message });
      else setErrors({ form: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  const domainName = domains.find((d) => d.id === domainId)?.name ?? "";

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (busy) return;
        onOpenChange(o);
        if (!o) reset();
      }}
    >
      <DialogContent className="max-w-lg">
        <form onSubmit={submit} className="space-y-4" noValidate>
          <DialogHeader>
            <DialogTitle>New mailbox</DialogTitle>
            <DialogDescription>A real address with its own storage. It works in Yunori and in any IMAP/SMTP mail app.</DialogDescription>
          </DialogHeader>

          <fieldset className="space-y-1.5">
            <legend className="mb-1.5 text-[13px] font-medium text-foreground/90">Address</legend>
            <AddressInput
              id="mb-local"
              local={local}
              onLocal={setLocal}
              domainId={domainId}
              onDomain={setDomainId}
              domains={domains}
              placeholder="name"
              autoFocus
              invalid={!!errors.local}
              describedBy={errors.local ? "mb-local-err" : undefined}
            />
            <FieldError id="mb-local-err">{errors.local ?? errors.domainId}</FieldError>
            {local && !errors.local && (
              <p className="text-xs text-muted-foreground">
                Will be <span className="font-mono text-foreground/85">{local.trim().toLowerCase()}@{domainName}</span>
              </p>
            )}
          </fieldset>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label="Display name" htmlFor="mb-name" optional>
              <Input id="mb-name" value={displayName} onChange={(e) => setDisplayName(e.target.value)} placeholder="Ren Ishikawa" />
            </Field>
            <Field label="Storage limit" htmlFor="mb-quota">
              <Select id="mb-quota" value={quota} onChange={(e) => setQuota(e.target.value)}>
                {QUOTAS.map((q) => (
                  <option key={q.label} value={q.mb === null ? "" : String(q.mb)}>
                    {q.label}
                  </option>
                ))}
              </Select>
            </Field>
          </div>

          <fieldset>
            <legend className="mb-2 text-[13px] font-medium text-foreground/90">How they'll get in</legend>
            <div className="grid gap-2">
              <RadioCard
                compact
                name="mb-pw"
                value="invite"
                checked={mode === "invite"}
                onChange={() => setMode("invite")}
                title="Send them an invite link"
                badge={<Badge tone="ok">Recommended</Badge>}
                description="They choose their own password, then set up their devices. The link works once, for 7 days."
              />
              <RadioCard
                compact
                name="mb-pw"
                value="generate"
                checked={mode === "generate"}
                onChange={() => setMode("generate")}
                title="Generate a strong password"
                description="Shown once after you create the mailbox; you pass it on."
              />
              <RadioCard compact name="mb-pw" value="manual" checked={mode === "manual"} onChange={() => setMode("manual")} title="Set one myself" />
            </div>
            {mode === "manual" && (
              <div className="mt-3 space-y-1.5">
                <PasswordInput
                  aria-label="Password"
                  autoComplete="new-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  aria-invalid={!!errors.password}
                  aria-describedby={errors.password ? "mb-pw-err" : "mb-pw-strength"}
                />
                {errors.password ? <FieldError id="mb-pw-err">{errors.password}</FieldError> : <StrengthHint password={password} id="mb-pw-strength" />}
              </div>
            )}
          </fieldset>

          <FormError>{errors.form}</FormError>

          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={!local.trim()} loading={busy}>
              {busy ? "Creating…" : "Create mailbox"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function EditMailboxDialog({ mailbox, onClose, onSaved }: { mailbox: Mailbox; onClose: () => void; onSaved: () => void }) {
  const currentMb = mailbox.quotaBytes === null ? "" : String(Math.round(mailbox.quotaBytes / 1024 ** 2));
  const [displayName, setDisplayName] = useState(mailbox.displayName ?? "");
  const [quota, setQuota] = useState(currentMb);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const presets = QUOTAS.map((q) => (q.mb === null ? "" : String(q.mb)));

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.updateMailbox(mailbox.id, { displayName: displayName.trim() || null, quotaMb: quota === "" ? null : Number(quota) });
      toast.success(`${mailbox.address} updated.`);
      onSaved();
      onClose();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open onOpenChange={(o) => !o && !busy && onClose()}>
      <DialogContent>
        <form onSubmit={submit} className="space-y-4">
          <DialogHeader>
            <DialogTitle>Edit {mailbox.address}</DialogTitle>
            <DialogDescription>The address itself can't change. Create a new mailbox or an alias instead.</DialogDescription>
          </DialogHeader>
          <Field label="Display name" htmlFor="ed-name">
            <Input id="ed-name" value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
          </Field>
          <Field
            label="Storage limit"
            htmlFor="ed-quota"
            hint={`Using ${fmtBytes(mailbox.usedBytes ?? 0)} now. Lowering it never deletes mail; once it's full, new mail is refused until there's room.`}
          >
            <Select id="ed-quota" value={quota} onChange={(e) => setQuota(e.target.value)}>
              {!presets.includes(currentMb) && <option value={currentMb}>Current ({fmtBytes(mailbox.quotaBytes)})</option>}
              {QUOTAS.map((q) => (
                <option key={q.label} value={q.mb === null ? "" : String(q.mb)}>
                  {q.label}
                </option>
              ))}
            </Select>
          </Field>
          <FormError>{error}</FormError>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              {busy ? "Saving…" : "Save"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Four numbers above the table: count, storage, invites waiting, nearly full. */
function Summary({ mailboxes }: { mailboxes: Mailbox[] }) {
  const used = mailboxes.reduce((n, m) => n + (m.usedBytes ?? 0), 0);
  const invites = mailboxes.filter((m) => m.inviteExpiresAt).length;
  const full = mailboxes.filter((m) => (pct(m.usedBytes, m.quotaBytes) ?? 0) >= 90).length;
  return (
    <div className="mb-6 grid grid-cols-2 gap-3 sm:gap-4 xl:grid-cols-4">
      <StatCard label="Mailboxes" icon={Inbox} value={fmtNumber(mailboxes.length)} hint={`${mailboxes.filter((m) => m.role === "admin").length} with admin role`} />
      <StatCard label="Storage used" icon={HardDrive} value={fmtBytes(used)} hint="Across every mailbox" />
      <StatCard label="Invites pending" icon={Link2} value={fmtNumber(invites)} hint={invites ? "Waiting for a password" : "None waiting"} tone={invites ? "info" : undefined} />
      <StatCard label="Nearly full" icon={Gauge} value={fmtNumber(full)} hint={full ? "Over 90% of their limit" : "Everyone has room"} tone={full ? "warn" : undefined} />
    </div>
  );
}
