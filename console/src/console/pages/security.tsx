import { useMemo, useState } from "react";
import { Search, ShieldAlert, ShieldCheck } from "lucide-react";
import { Input } from "@/design/vendor/ui/input";
import { Card, SectionLabel, Skeleton } from "@/design/vendor/admin/ui";
import { Pager } from "@/design/vendor/admin/data-table";
import { api } from "@/api/client";
import type { AdminUser, AuditEntry } from "@/api/types";
import { DataTable, LoadError, Page, usePageTitle, type Column } from "@/components/kit";
import { StatusBadge } from "@/components/status";
import { fmtDate, fmtDateTime, fmtNumber, timeAgo } from "@/lib/format";
import { useApi } from "@/lib/use-api";
import { IpAccessSection } from "../ip-access";
import { useSession } from "../session";
import { roleLabel } from "../shell";

const PAGE_SIZE = 25;

/** Admins and the append-only, hash-chained audit ledger. */
export default function SecurityPage() {
  usePageTitle("Security");
  const { session } = useSession();
  const audit = useApi("audit", api.audit);
  // GET /admins is optional for now — fall back to the signed-in admin.
  const admins = useApi<AdminUser[]>("admins", () => api.admins().catch(() => (session.admin ? [session.admin] : [])));
  const [q, setQ] = useState("");
  const [page, setPage] = useState(1);

  const rows = useMemo(() => {
    const needle = q.trim().toLowerCase();
    const all = audit.data?.entries ?? [];
    return needle ? all.filter((e) => [e.action, e.actor, e.target ?? "", e.detail ?? ""].some((v) => v.toLowerCase().includes(needle))) : all;
  }, [audit.data, q]);
  const paged = rows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE);

  const columns: Column<AuditEntry>[] = [
    {
      key: "id",
      header: "#",
      cell: (e) => <span className="text-xs tabular-nums text-muted-foreground">{e.id}</span>,
    },
    {
      key: "at",
      header: "When",
      cell: (e) => (
        <time dateTime={e.at} title={fmtDateTime(e.at)} className="whitespace-nowrap text-xs text-muted-foreground">
          {timeAgo(e.at)}
        </time>
      ),
    },
    {
      key: "action",
      header: "Action",
      cell: (e) => <span className="rounded-md border border-border/60 bg-card/60 px-1.5 py-0.5 font-mono text-[11px] text-foreground/85">{e.action}</span>,
    },
    {
      key: "target",
      header: "Target",
      cell: (e) => <span className="text-xs text-foreground/85">{e.target ?? "—"}</span>,
    },
    {
      key: "actor",
      header: "By",
      hideBelowMd: true,
      cell: (e) => <span className="text-xs text-muted-foreground">{e.actor}</span>,
    },
    {
      key: "detail",
      header: "Detail",
      hideBelowMd: true,
      cell: (e) => <span className="text-xs text-muted-foreground">{e.detail ?? "—"}</span>,
    },
  ];

  return (
    <Page title="Security" blurb="Who can administer this server, addresses blocked for failed sign-ins, and everything admins have done. Secrets and mail contents are never recorded.">
      <div className="grid gap-4 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <SectionLabel>Administrators</SectionLabel>
          {admins.data ? (
            <ul className="divide-y divide-border/50">
              {admins.data.map((a) => (
                <li key={a.id} className="flex flex-wrap items-center justify-between gap-2 py-2.5">
                  <div className="min-w-0">
                    <p className="truncate text-sm text-foreground/90">
                      {a.email}
                      {a.id === session.admin?.id && <span className="ml-2 text-[11px] text-muted-foreground">(you)</span>}
                    </p>
                    <p className="text-[11px] text-muted-foreground">
                      {roleLabel(a.role)} · since {fmtDate(a.createdAt)}
                    </p>
                  </div>
                  {a.totpEnabled ? <StatusBadge tone="ok">Two-step on</StatusBadge> : <StatusBadge tone="warn">Two-step off</StatusBadge>}
                </li>
              ))}
            </ul>
          ) : (
            <Skeleton className="h-16" />
          )}
          <p className="mt-3 border-t border-border/50 pt-3 text-[11px] leading-relaxed text-muted-foreground">
            Every admin signs in with a password and an authenticator code. Inviting more admins arrives in a later release.
          </p>
        </Card>
        <Card>
          <SectionLabel>Ledger integrity</SectionLabel>
          {audit.data ? (
            <div>
              {audit.data.chainOk ? (
                <StatusBadge tone="ok" icon={ShieldCheck}>
                  Chain intact
                </StatusBadge>
              ) : (
                <StatusBadge tone="bad" icon={ShieldAlert}>
                  Chain broken
                </StatusBadge>
              )}
              <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                {audit.data.chainOk
                  ? `All ${fmtNumber(audit.data.entries.length)} entries verified end to end. Each entry's hash covers the one before it, so an edited or deleted row would show here.`
                  : "An entry was changed or removed after it was written. Treat this server as compromised: check SSH access and restore from a backup you trust."}
              </p>
            </div>
          ) : (
            <Skeleton className="h-16" />
          )}
        </Card>
      </div>

      <IpAccessSection />

      <div className="mb-3 mt-8 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">Audit ledger</h2>
        <div className="relative w-full sm:w-64">
          <Search aria-hidden className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={q}
            onChange={(e) => {
              setQ(e.target.value);
              setPage(1);
            }}
            placeholder="Filter by action, person, target…"
            aria-label="Filter the audit ledger"
            className="h-9 pl-8"
          />
        </div>
      </div>
      {audit.error && !audit.data ? (
        <LoadError error={audit.error} onRetry={() => void audit.reload()} what="The audit ledger" />
      ) : (
        <DataTable
          columns={columns}
          rows={paged}
          rowKey={(e) => String(e.id)}
          loading={audit.loading && !audit.data}
          emptyTitle={q ? "No entries match" : "The ledger is empty"}
          emptyHint={q ? "Try a different filter." : "Every admin action is recorded here permanently."}
          footer={rows.length > PAGE_SIZE ? <Pager page={page} pageSize={PAGE_SIZE} total={rows.length} onPage={setPage} /> : undefined}
        />
      )}
    </Page>
  );
}
