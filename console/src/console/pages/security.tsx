import { useMemo, useState } from "react";
import { ScrollText, ShieldAlert, ShieldCheck, Users } from "lucide-react";
import { Avatar, Chip } from "@/ui/badge";
import { Card, CardBody, CardHeader, SectionHeader } from "@/ui/card";
import { Pager } from "@/ui/data-table";
import { Skeleton } from "@/ui/feedback";
import { SearchInput } from "@/ui/segmented";
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
      className: "w-14",
      mobile: "hidden",
      cell: (e) => <span className="text-xs tabular-nums text-muted-foreground">{e.id}</span>,
    },
    {
      key: "at",
      header: "When",
      cell: (e) => (
        <time dateTime={e.at} title={fmtDateTime(e.at)} className="whitespace-nowrap text-[13px] text-muted-foreground">
          {timeAgo(e.at)}
        </time>
      ),
    },
    {
      key: "action",
      header: "Action",
      mobile: "title",
      cell: (e) => <Chip>{e.action}</Chip>,
    },
    {
      key: "target",
      header: "Target",
      cell: (e) => <span className="break-all text-[13px] text-foreground">{e.target ?? "—"}</span>,
    },
    {
      key: "actor",
      header: "By",
      hideBelowMd: true,
      cell: (e) => <span className="text-[13px] text-muted-foreground">{e.actor}</span>,
    },
    {
      key: "detail",
      header: "Detail",
      hideBelowMd: true,
      cell: (e) => <span className="text-[13px] text-muted-foreground">{e.detail ?? "—"}</span>,
    },
  ];

  return (
    <Page title="Security" blurb="Who can administer this server, addresses blocked for failed sign-ins, and everything admins have done. Secrets and mail contents are never recorded.">
      <div className="grid gap-4 xl:grid-cols-12">
        <Card padded={false} className="xl:col-span-8" aria-labelledby="sec-admins">
          <CardHeader id="sec-admins" title="Administrators" icon={Users} description="Every admin signs in with a password and an authenticator code." divider />
          <CardBody className="pb-2">
          {admins.data ? (
            <ul className="divide-y divide-border/60">
              {admins.data.map((a) => (
                <li key={a.id} className="flex flex-wrap items-center justify-between gap-2 py-3">
                  <div className="flex min-w-0 items-center gap-3">
                    <Avatar name={a.email} />
                    <div className="min-w-0">
                    <p className="truncate text-sm font-medium text-foreground">
                      {a.email}
                      {a.id === session.admin?.id && <span className="ml-2 text-xs font-normal text-muted-foreground">(you)</span>}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {roleLabel(a.role)} · since {fmtDate(a.createdAt)}
                    </p>
                    </div>
                  </div>
                  {a.totpEnabled ? <StatusBadge tone="ok">Two-step on</StatusBadge> : <StatusBadge tone="warn">Two-step off</StatusBadge>}
                </li>
              ))}
            </ul>
          ) : (
            <Skeleton className="h-16" />
          )}
          <p className="border-t border-border/70 py-3 text-xs leading-relaxed text-muted-foreground">Inviting more admins arrives in a later release.</p>
          </CardBody>
        </Card>
        <Card padded={false} className="xl:col-span-4" aria-labelledby="sec-ledger">
          <CardHeader id="sec-ledger" title="Ledger integrity" icon={ScrollText} />
          <CardBody>
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
              <p className="mt-3 text-[13px] leading-relaxed text-muted-foreground">
                {audit.data.chainOk
                  ? `All ${fmtNumber(audit.data.entries.length)} entries verified end to end. Each entry's hash covers the one before it, so an edited or deleted row would show here.`
                  : "An entry was changed or removed after it was written. Treat this server as compromised: check SSH access and restore from a backup you trust."}
              </p>
            </div>
          ) : (
            <Skeleton className="h-16" />
          )}
          </CardBody>
        </Card>
      </div>

      <IpAccessSection />

      <SectionHeader
        className="mt-10"
        title="Audit ledger"
        description="Append-only and hash-chained. Secrets and mail contents are never recorded."
        actions={
          <SearchInput
            value={q}
            onValueChange={(v) => {
              setQ(v);
              setPage(1);
            }}
            placeholder="Filter by action, person, target…"
            aria-label="Filter the audit ledger"
          />
        }
      />
      {audit.error && !audit.data ? (
        <LoadError error={audit.error} onRetry={() => void audit.reload()} what="The audit ledger" />
      ) : (
        <DataTable
          columns={columns}
          rows={paged}
          rowKey={(e) => String(e.id)}
          loading={audit.loading && !audit.data}
          caption="Audit ledger"
          emptyIcon={ScrollText}
          emptyTitle={q ? "No entries match" : "The ledger is empty"}
          emptyHint={q ? "Try a different filter." : "Every admin action is recorded here permanently."}
          footer={rows.length > PAGE_SIZE ? <Pager page={page} pageSize={PAGE_SIZE} total={rows.length} onPage={setPage} /> : undefined}
        />
      )}
    </Page>
  );
}
