import { useEffect, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router-dom";
import { ChevronRight, Globe, Plus } from "lucide-react";
import { Button } from "@/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/ui/overlay";
import { Input, Label } from "@/ui/form";
import { Badge } from "@/ui/badge";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { Domain } from "@/api/types";
import { DataTable, FieldError, LoadError, Page, usePageTitle, type Column } from "@/components/kit";
import { CHECK_META, StatusBadge } from "@/components/status";
import { fmtNumber, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";
import { useShell } from "../shell";

export const DNS_LABEL: Record<Domain["dnsStatus"], string> = {
  pass: "DNS ready",
  warn: "Needs a tweak",
  fail: "Records missing",
  info: "DNS ready",
  unknown: "Not checked",
};

/** Every domain this server receives mail for, with its DNS state in words. */
export default function DomainsPage() {
  usePageTitle("Domains");
  const navigate = useNavigate();
  const [params, setParams] = useSearchParams();
  const { data, error, loading, reload } = useApi("domains", api.domains);
  const [addOpen, setAddOpen] = useState(params.get("new") === "1");

  useEffect(() => {
    if (params.get("new") === "1") {
      setAddOpen(true);
      setParams((p) => {
        const n = new URLSearchParams(p);
        n.delete("new");
        return n;
      }, { replace: true });
    }
  }, [params, setParams]);

  const columns: Column<Domain>[] = [
    {
      key: "name",
      header: "Domain",
      cell: (d) => (
        <div className="flex min-w-0 items-center gap-3">
          <span aria-hidden className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-border bg-muted/50 text-icon">
            <Globe className="h-4 w-4" strokeWidth={1.75} />
          </span>
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <Link
              to={`/console/domains/${d.id}`}
              className="rounded-sm text-sm font-medium text-foreground underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
            >
              {d.name}
            </Link>
            {d.isPrimary && <Badge>Primary</Badge>}
            {!d.enabled && <StatusBadge tone="muted" size="xs">Disabled</StatusBadge>}
          </div>
        </div>
      ),
    },
    {
      key: "dns",
      header: "DNS",
      cell: (d) => {
        const m = CHECK_META[d.dnsStatus];
        return (
          <div className="flex flex-col items-start gap-1">
            <StatusBadge tone={m.tone} icon={m.icon}>
              {DNS_LABEL[d.dnsStatus]}
            </StatusBadge>
            <span className="text-xs text-muted-foreground">{d.dnsCheckedAt ? `checked ${timeAgo(d.dnsCheckedAt)}` : "never checked"}</span>
          </div>
        );
      },
    },
    {
      key: "mailboxes",
      header: "Mailboxes",
      cell: (d) => (
        <Link
          to={`/console/mailboxes?domain=${d.id}`}
          className="rounded-sm text-[13px] tabular-nums text-foreground underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
        >
          {fmtNumber(d.mailboxCount)}
          <span className="sr-only"> mailboxes on {d.name}</span>
        </Link>
      ),
    },
    {
      key: "catchAll",
      header: "Catch-all",
      hideBelowMd: true,
      cell: (d) => <span className="text-[13px] text-muted-foreground">{d.catchAll ?? "Off"}</span>,
    },
    {
      key: "created",
      header: "Added",
      hideBelowMd: true,
      cell: (d) => <span className="text-[13px] text-muted-foreground">{timeAgo(d.createdAt)}</span>,
    },
    {
      key: "open",
      header: <span className="sr-only">Open</span>,
      className: "w-10",
      mobile: "hidden",
      cell: () => <ChevronRight aria-hidden className="h-4 w-4 text-muted-foreground" />,
    },
  ];

  return (
    <Page
      title="Domains"
      blurb={
        data
          ? `${fmtNumber(data.length)} domain${data.length === 1 ? "" : "s"}. Mail for a domain arrives once its DNS records resolve.`
          : "Mail for a domain arrives once its DNS records resolve."
      }
      actions={
        <Button size="sm" onClick={() => setAddOpen(true)}>
          <Plus className="h-4 w-4" />
          Add domain
        </Button>
      }
    >
      {error && !data ? (
        <LoadError error={error} onRetry={() => void reload()} what="Domains" />
      ) : (
        <DataTable
          columns={columns}
          rows={data ?? []}
          rowKey={(d) => d.id}
          loading={loading && !data}
          caption="Domains"
          onRowClick={(d) => navigate(`/console/domains/${d.id}`)}
          emptyIcon={Globe}
          emptyAction={
            <Button size="sm" onClick={() => setAddOpen(true)}>
              <Plus className="h-4 w-4" />
              Add domain
            </Button>
          }
          emptyTitle="No domains yet"
          emptyHint="Add the domain you want addresses on — Yunori creates its DKIM keys and the full record set for you."
        />
      )}

      <AddDomainDialog
        open={addOpen}
        onOpenChange={setAddOpen}
        onCreated={(d) => {
          void reload();
          navigate(`/console/domains/${d.id}`);
        }}
      />
    </Page>
  );
}

function AddDomainDialog({ open, onOpenChange, onCreated }: { open: boolean; onOpenChange: (o: boolean) => void; onCreated: (d: Domain) => void }) {
  const { refreshHealth } = useShell();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const clean = name.trim().toLowerCase().replace(/^https?:\/\//, "").replace(/\/.*$/, "").replace(/^.*@/, "");
    if (!/^(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/.test(clean)) {
      setError("Enter a domain like example.org.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const d = await api.createDomain({ name: clean });
      toast.success(`${d.name} added. Publish its records next.`);
      refreshHealth();
      onOpenChange(false);
      setName("");
      onCreated(d);
    } catch (err) {
      setError(err instanceof ApiRequestError ? err.message : errorMessage(err));
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
        if (!o) setError(null);
      }}
    >
      <DialogContent>
        <form onSubmit={submit} className="space-y-4">
          <DialogHeader>
            <DialogTitle>Add a domain</DialogTitle>
            <DialogDescription>
              Yunori generates DKIM keys and the full record set. Nothing changes for this domain's mail until you
              publish the records at your DNS provider.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="domain-name">Domain</Label>
            <Input
              id="domain-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="example.org"
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              autoFocus
              aria-invalid={!!error}
              aria-describedby={error ? "domain-err" : undefined}
            />
            <FieldError id="domain-err">{error}</FieldError>
          </div>
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !name.trim()}>
              {busy ? "Adding…" : "Add domain"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
