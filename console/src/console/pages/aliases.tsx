import { useEffect, useMemo, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { ArrowRight, Plus, Search, Trash2 } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/design/vendor/ui/dialog";
import { Input } from "@/design/vendor/ui/input";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { Alias, Domain, Mailbox } from "@/api/types";
import { ConfirmAction } from "@/components/confirm";
import { DataTable, FieldError, LoadError, Page, selectClass, usePageTitle, type Column } from "@/components/kit";
import { StatusBadge } from "@/components/status";
import { fmtNumber } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";

/** Extra addresses that deliver into an existing mailbox. */
export default function AliasesPage() {
  usePageTitle("Aliases");
  const [params, setParams] = useSearchParams();
  const { data, error, loading, reload } = useApi("aliases", async () => {
    const [aliases, mailboxes, domains] = await Promise.all([api.aliases(), api.mailboxes(), api.domains()]);
    return { aliases, mailboxes, domains };
  });
  const [q, setQ] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const [deleting, setDeleting] = useState<Alias | null>(null);

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

  const rows = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return (data?.aliases ?? [])
      .filter((a) => !needle || a.address.includes(needle) || a.mailboxAddress.includes(needle))
      .sort((a, b) => a.address.localeCompare(b.address));
  }, [data, q]);

  const columns: Column<Alias>[] = [
    {
      key: "address",
      header: "Alias",
      cell: (a) => <span className={cn("text-sm font-medium", a.enabled ? "text-foreground/90" : "text-muted-foreground")}>{a.address}</span>,
    },
    {
      key: "target",
      header: "Delivers to",
      cell: (a) => (
        <span className="inline-flex items-center gap-1.5 text-xs text-foreground/80">
          <ArrowRight aria-hidden className="h-3 w-3 text-muted-foreground" />
          {a.mailboxAddress}
        </span>
      ),
    },
    {
      key: "status",
      header: "Status",
      hideBelowMd: true,
      cell: (a) => (a.enabled ? <StatusBadge tone="ok">Active</StatusBadge> : <StatusBadge tone="muted">Paused</StatusBadge>),
    },
    {
      key: "actions",
      header: <span className="sr-only">Actions</span>,
      className: "w-12 text-right",
      cell: (a) => (
        <button
          type="button"
          onClick={() => setDeleting(a)}
          aria-label={`Delete alias ${a.address}`}
          title="Delete alias"
          className="grid h-8 w-8 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      ),
    },
  ];

  return (
    <Page
      title="Aliases"
      blurb={
        data
          ? `${fmtNumber(data.aliases.length)} alias${data.aliases.length === 1 ? "" : "es"}. An alias is another address for an existing mailbox — no separate password, no separate storage.`
          : "An alias is another address for an existing mailbox."
      }
      actions={
        <Button size="sm" onClick={() => setCreateOpen(true)} disabled={!data || data.mailboxes.length === 0}>
          <Plus className="h-4 w-4" />
          New alias
        </Button>
      }
    >
      <div className="mb-4 flex flex-wrap items-center gap-2">
        <div className="relative min-w-52 flex-1 sm:max-w-xs">
          <Search aria-hidden className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search aliases…" aria-label="Search aliases" className="h-9 pl-8" />
        </div>
      </div>

      {error && !data ? (
        <LoadError error={error} onRetry={() => void reload()} what="Aliases" />
      ) : (
        <DataTable
          columns={columns}
          rows={rows}
          rowKey={(a) => a.address}
          loading={loading && !data}
          emptyTitle={q ? "No aliases match" : "No aliases yet"}
          emptyHint={q ? "Try another search." : "Use one for info@, billing@ or a role address that several people read through one mailbox."}
        />
      )}

      {data && (
        <CreateAliasDialog
          open={createOpen}
          onOpenChange={setCreateOpen}
          domains={data.domains}
          mailboxes={data.mailboxes}
          onCreated={(a) => {
            toast.success(`${a.address} now delivers to ${a.mailboxAddress}.`);
            void reload();
          }}
        />
      )}

      <ConfirmAction
        open={!!deleting}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={`Delete ${deleting?.address ?? ""}?`}
        description={`Mail sent to this address will be refused. Mail already delivered to ${deleting?.mailboxAddress ?? "the mailbox"} stays where it is.`}
        confirmLabel="Delete alias"
        destructive
        onConfirm={async () => {
          const a = deleting!;
          await api.deleteAlias(a.address);
          toast.success(`${a.address} was deleted.`);
          void reload();
        }}
      />
    </Page>
  );
}

function CreateAliasDialog({
  open,
  onOpenChange,
  domains,
  mailboxes,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  domains: Domain[];
  mailboxes: Mailbox[];
  onCreated: (a: Alias) => void;
}) {
  const primary = domains.find((d) => d.isPrimary)?.id ?? domains[0]?.id ?? "";
  const [local, setLocal] = useState("");
  const [domainId, setDomainId] = useState(primary);
  const [mailboxId, setMailboxId] = useState(mailboxes[0]?.id ?? "");
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});

  const reset = () => {
    setLocal("");
    setErrors({});
  };

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const local_ = local.trim().toLowerCase();
    if (!/^[a-z0-9](?:[a-z0-9._+-]{0,62}[a-z0-9])?$/.test(local_)) {
      setErrors({ local: "Use letters, numbers, dots, dashes or underscores." });
      return;
    }
    setBusy(true);
    setErrors({});
    try {
      const a = await api.createAlias({ local: local_, domainId, mailboxId });
      onOpenChange(false);
      reset();
      onCreated(a);
    } catch (err) {
      // 409: it would shadow a mailbox (or already exists) — the server says which.
      if (err instanceof ApiRequestError && (err.status === 409 || err.field === "local")) setErrors({ local: err.message });
      else if (err instanceof ApiRequestError && err.field) setErrors({ [err.field]: err.message });
      else setErrors({ form: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (busy) return;
        onOpenChange(o);
        if (!o) reset();
      }}
    >
      <DialogContent>
        <form onSubmit={submit} className="space-y-4" noValidate>
          <DialogHeader>
            <DialogTitle>New alias</DialogTitle>
            <DialogDescription>Mail to the alias lands in the mailbox you pick. An alias can't reuse a mailbox's address.</DialogDescription>
          </DialogHeader>
          <fieldset className="space-y-1.5">
            <legend className="mb-1.5 text-xs font-medium text-foreground/80">Alias address</legend>
            <div className="flex items-stretch">
              <Input
                aria-label="Name before the @"
                value={local}
                onChange={(e) => setLocal(e.target.value)}
                placeholder="info"
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                autoFocus
                className="rounded-r-none"
                aria-invalid={!!errors.local}
                aria-describedby={errors.local ? "al-local-err" : undefined}
              />
              <span aria-hidden className="grid place-items-center border-y border-input bg-muted/40 px-2 text-sm text-muted-foreground">
                @
              </span>
              <select aria-label="Domain" className={cn(selectClass, "w-auto max-w-[55%] rounded-l-none")} value={domainId} onChange={(e) => setDomainId(e.target.value)}>
                {domains.map((d) => (
                  <option key={d.id} value={d.id}>
                    {d.name}
                  </option>
                ))}
              </select>
            </div>
            <FieldError id="al-local-err">{errors.local ?? errors.domainId}</FieldError>
          </fieldset>
          <div className="space-y-1.5">
            <label htmlFor="al-target" className="block text-xs font-medium text-foreground/80">
              Deliver to
            </label>
            <select id="al-target" className={selectClass} value={mailboxId} onChange={(e) => setMailboxId(e.target.value)}>
              {mailboxes.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.address}
                  {m.displayName ? ` — ${m.displayName}` : ""}
                </option>
              ))}
            </select>
            <FieldError id="al-target-err">{errors.mailboxId}</FieldError>
          </div>
          {errors.form && (
            <p role="alert" className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs">
              {errors.form}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !local.trim()}>
              {busy ? "Creating…" : "Create alias"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
