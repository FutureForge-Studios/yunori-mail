import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { ArrowLeft, RefreshCw, Trash2 } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Switch } from "@/design/vendor/ui/switch";
import { Card, EmptyState, SectionLabel, Skeleton, selectClass } from "@/design/vendor/admin/ui";
import { api, errorMessage } from "@/api/client";
import type { DnsRecord, Domain, DomainDnsResponse, Mailbox } from "@/api/types";
import { ConfirmAction } from "@/components/confirm";
import { CopyButton } from "@/components/copy-button";
import { FilterPills, LoadError, Page, usePageTitle } from "@/components/kit";
import { CheckPill, StatusBadge } from "@/components/status";
import { fmtDateTime, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";
import { useCrumb, useShell } from "../shell";

interface Loaded {
  domain: Domain | null;
  dns: DomainDnsResponse | null;
  mailboxes: Mailbox[];
}

/** One domain: its DNS records in plain language, catch-all, enable, delete. */
export default function DomainDetailPage() {
  const { domainId = "" } = useParams();
  const { refreshHealth } = useShell();
  const { data, error, reload, setData } = useApi<Loaded>(`domain:${domainId}`, async () => {
    const domains = await api.domains();
    const domain = domains.find((d) => d.id === domainId) ?? null;
    if (!domain) return { domain: null, dns: null, mailboxes: [] };
    const [dns, mailboxes] = await Promise.all([api.dns(domainId), api.mailboxes(domainId)]);
    return { domain, dns, mailboxes };
  });
  const [checking, setChecking] = useState(false);
  const [filter, setFilter] = useState<"all" | "attention">("all");

  usePageTitle(data?.domain?.name ?? "Domain");
  useCrumb(data?.domain?.name);

  if (error && !data) {
    return (
      <Page title="Domain">
        <LoadError error={error} onRetry={() => void reload()} what="This domain" />
      </Page>
    );
  }
  if (!data) {
    return (
      <Page title="Domain" blurb="Loading…">
        <Skeleton className="mb-4 h-10" />
        <Skeleton className="h-[28rem]" />
      </Page>
    );
  }
  if (!data.domain || !data.dns) {
    return (
      <Page title="Domain not found">
        <EmptyState
          title="This domain isn't on the server"
          hint="It may have been deleted. Domains are listed on the Domains page."
          action={
            <Button asChild variant="outline" size="sm">
              <Link to="/console/domains">
                <ArrowLeft className="h-3.5 w-3.5" />
                All domains
              </Link>
            </Button>
          }
        />
      </Page>
    );
  }

  const { domain, dns, mailboxes } = data;
  const required = dns.records.filter((r) => r.required);
  const requiredOk = required.filter((r) => r.status === "pass" || r.status === "info").length;
  const attention = dns.records.filter((r) => r.status === "fail" || r.status === "warn");
  const shown = sortRecords(filter === "all" ? dns.records : attention);

  async function checkNow() {
    setChecking(true);
    try {
      const fresh = await api.checkDns(domainId);
      const domains = await api.domains();
      setData((d) => (d ? { ...d, dns: fresh, domain: domains.find((x) => x.id === domainId) ?? d.domain } : d));
      const missing = fresh.records.filter((r) => r.required && r.status === "fail").length;
      toast[missing ? "info" : "success"](
        missing ? `${missing} required record${missing === 1 ? " is" : "s are"} still missing. DNS changes can take a while to spread.` : "Every required record resolves.",
      );
      refreshHealth();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setChecking(false);
    }
  }

  return (
    <Page
      title={domain.name}
      blurb={`${domain.isPrimary ? "Primary domain · " : ""}${domain.mailboxCount} mailbox${domain.mailboxCount === 1 ? "" : "es"} · ${
        dns.checkedAt ? `checked ${timeAgo(dns.checkedAt)}` : "not checked yet"
      }`}
      actions={
        <>
          <CopyButton value={dns.zoneFile} label="zone file" showText text="Copy zone file" />
          <Button size="sm" onClick={() => void checkNow()} disabled={checking}>
            <RefreshCw className={cn("h-3.5 w-3.5", checking && "animate-spin motion-reduce:animate-none")} />
            {checking ? "Checking…" : "Check now"}
          </Button>
        </>
      }
    >
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap items-center gap-2">
          <CheckPill
            status={requiredOk === required.length ? "pass" : required.some((r) => r.status === "fail") ? "fail" : "warn"}
            label={`${requiredOk} of ${required.length} required records resolve`}
          />
          {attention.length > 0 && (
            <span className="text-xs text-muted-foreground">
              {attention.length} record{attention.length === 1 ? "" : "s"} need{attention.length === 1 ? "s" : ""} attention
            </span>
          )}
        </div>
        <FilterPills
          label="Filter records"
          value={filter}
          onChange={setFilter}
          options={[
            { value: "all", label: `All ${dns.records.length}` },
            { value: "attention", label: `Needs attention ${attention.length}` },
          ]}
        />
      </div>

      <DnsTable records={shown} />
      <p className="mt-2 text-[11px] text-muted-foreground">
        “Found” is what public resolvers return right now{dns.checkedAt ? ` (as of ${fmtDateTime(dns.checkedAt)})` : ""}. Values are copied
        exactly as they must be published; long TXT values are split automatically in the zone file.
      </p>

      <div className="mt-8 grid gap-4 lg:grid-cols-2">
        <CatchAllCard domain={domain} mailboxes={mailboxes} onSaved={(d) => setData((x) => (x ? { ...x, domain: d } : x))} />
        <StatusCard domain={domain} onSaved={(d) => setData((x) => (x ? { ...x, domain: d } : x))} />
      </div>

      <DangerZone domain={domain} />
    </Page>
  );
}

const STATUS_ORDER = { fail: 0, warn: 1, unknown: 2, info: 3, pass: 4 } as const;
function sortRecords(records: DnsRecord[]): DnsRecord[] {
  return [...records].sort((a, b) => Number(b.required) - Number(a.required) || STATUS_ORDER[a.status] - STATUS_ORDER[b.status]);
}

// ── DNS table ────────────────────────────────────────────────────────────────

function DnsTable({ records }: { records: DnsRecord[] }) {
  if (records.length === 0) return <EmptyState title="Nothing needs attention" hint="Every record resolves as expected." />;
  return (
    <div className="overflow-hidden rounded-xl border border-border/60">
      <table className="w-full text-sm">
        <caption className="sr-only">DNS records for this domain: what to publish, what resolvers return, and status</caption>
        <thead className="max-md:hidden">
          <tr className="border-b border-border/60 bg-muted/20">
            {["Record", "Value", "Status"].map((h, i) => (
              <th
                key={h}
                scope="col"
                className={cn(
                  "px-4 py-2.5 text-left text-[11px] font-medium uppercase tracking-[0.1em] text-muted-foreground",
                  i === 0 && "w-[34%]",
                  i === 2 && "w-36",
                )}
              >
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {records.map((r) => (
            <DnsRow key={r.id} record={r} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function DnsRow({ record: r }: { record: DnsRecord }) {
  const [expanded, setExpanded] = useState(false);
  const long = r.expected.length > 110;
  const matches = r.actual.length > 0 && r.actual.includes(r.expected);
  return (
    <tr className="border-b border-border/40 bg-card/30 align-top last:border-0 max-md:block max-md:py-2">
      <td className="px-4 py-3.5 max-md:block max-md:py-1.5">
        <div className="flex items-center gap-2">
          <span className="w-12 shrink-0 rounded border border-border/70 px-1 py-px text-center font-mono text-[10px] text-foreground/80">{r.type}</span>
          <span className="min-w-0 break-all font-mono text-xs text-foreground/90">{r.name}</span>
          <CopyButton value={r.name} label={`name ${r.name}`} className="h-6 w-6" />
        </div>
        <p className="mt-1.5 text-xs leading-relaxed text-muted-foreground">{r.purpose}</p>
      </td>
      <td className="px-4 py-3.5 max-md:block max-md:py-1.5">
        <div className="flex items-start gap-2">
          <span className="w-14 shrink-0 pt-px text-[11px] text-muted-foreground">Publish</span>
          <div className="min-w-0 flex-1">
            <code className={cn("block break-all font-mono text-xs text-foreground/90", long && !expanded && "line-clamp-2")}>{r.expected}</code>
            {long && (
              <button
                type="button"
                onClick={() => setExpanded((v) => !v)}
                aria-expanded={expanded}
                className="mt-0.5 rounded-sm text-[11px] text-muted-foreground underline underline-offset-2 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              >
                {expanded ? "Show less" : `Show all ${r.expected.length} characters`}
              </button>
            )}
          </div>
          <CopyButton value={r.expected} label={`${r.type} value for ${r.name}`} className="h-6 w-6" />
        </div>
        <div className="mt-2 flex items-start gap-2">
          <span className="w-14 shrink-0 pt-px text-[11px] text-muted-foreground">Found</span>
          <div className="min-w-0 flex-1">
            {r.actual.length === 0 ? (
              <span className="text-xs text-muted-foreground">Nothing published yet</span>
            ) : matches ? (
              <span className="text-xs text-muted-foreground">Same as above</span>
            ) : (
              r.actual.map((a, i) => (
                <code key={i} className="block break-all font-mono text-xs text-foreground/75">
                  {a}
                </code>
              ))
            )}
          </div>
        </div>
        {r.remedy && (
          <p className="mt-2 rounded-md bg-muted/40 px-2.5 py-1.5 text-xs leading-relaxed text-foreground/85">
            <span className="font-medium">What to do: </span>
            {r.remedy}
          </p>
        )}
      </td>
      <td className="px-4 py-3.5 max-md:flex max-md:items-center max-md:gap-2 max-md:py-1.5">
        <CheckPill status={r.status} />
        <p className="mt-1.5 text-[11px] text-muted-foreground max-md:mt-0">{r.required ? "Required" : "Optional"}</p>
      </td>
    </tr>
  );
}

// ── Settings ─────────────────────────────────────────────────────────────────

function CatchAllCard({ domain, mailboxes, onSaved }: { domain: Domain; mailboxes: Mailbox[]; onSaved: (d: Domain) => void }) {
  const [value, setValue] = useState(domain.catchAll ?? "");
  const [busy, setBusy] = useState(false);
  useEffect(() => setValue(domain.catchAll ?? ""), [domain.catchAll]);
  const dirty = value !== (domain.catchAll ?? "");

  async function save() {
    setBusy(true);
    try {
      const d = await api.updateDomain(domain.id, { catchAll: value || null });
      onSaved(d);
      toast.success(value ? `Mail to unknown addresses at ${domain.name} now goes to ${value}.` : "Catch-all is off.");
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <SectionLabel>Catch-all</SectionLabel>
      <p className="-mt-1 text-xs leading-relaxed text-muted-foreground">
        Mail to an address that doesn't exist at {domain.name} is normally refused. A catch-all delivers it to one mailbox
        instead — handy, but it also collects spam sent to guessed addresses.
      </p>
      <div className="mt-4 flex flex-wrap items-end gap-2">
        <div className="min-w-56 flex-1 space-y-1.5">
          <label htmlFor="catch-all" className="block text-xs font-medium text-foreground/80">
            Deliver unknown addresses to
          </label>
          <select id="catch-all" className={selectClass} value={value} onChange={(e) => setValue(e.target.value)}>
            <option value="">Off — refuse them</option>
            {mailboxes.map((m) => (
              <option key={m.id} value={m.address}>
                {m.address}
              </option>
            ))}
          </select>
        </div>
        <Button size="sm" variant="outline" onClick={() => void save()} disabled={!dirty || busy} className="h-9">
          {busy ? "Saving…" : "Save"}
        </Button>
      </div>
    </Card>
  );
}

function StatusCard({ domain, onSaved }: { domain: Domain; onSaved: (d: Domain) => void }) {
  const [busy, setBusy] = useState(false);
  async function toggle(enabled: boolean) {
    setBusy(true);
    try {
      onSaved(await api.updateDomain(domain.id, { enabled }));
      toast.success(enabled ? `${domain.name} accepts mail again.` : `${domain.name} now refuses new mail. Existing mail is kept.`);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Card>
      <SectionLabel>Receiving</SectionLabel>
      <div className="flex items-start justify-between gap-4">
        <div>
          <label htmlFor="domain-enabled" className="text-sm text-foreground/90">
            Accept mail for {domain.name}
          </label>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
            {domain.isPrimary
              ? "The primary domain names this server, so it's always on."
              : "Turning this off refuses new mail for the domain. Mailboxes and their mail stay as they are."}
          </p>
        </div>
        <div className="flex items-center gap-2 pt-0.5">
          <span className="text-[11px] text-muted-foreground" aria-hidden>
            {domain.enabled ? "On" : "Off"}
          </span>
          <Switch id="domain-enabled" checked={domain.enabled} disabled={busy || domain.isPrimary} onCheckedChange={(v) => void toggle(v)} />
        </div>
      </div>
    </Card>
  );
}

function DangerZone({ domain }: { domain: Domain }) {
  const navigate = useNavigate();
  const { refreshHealth } = useShell();
  const [open, setOpen] = useState(false);
  if (domain.isPrimary) return null;
  return (
    <section className="mt-8 rounded-xl border border-border/60 p-5" aria-labelledby="danger-title">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id="danger-title" className="text-sm font-medium text-foreground/90">
            Delete {domain.name}
          </h2>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {domain.mailboxCount > 0
              ? `Only possible once its ${domain.mailboxCount} mailbox${domain.mailboxCount === 1 ? " is" : "es are"} deleted — mail is never removed silently.`
              : "Removes the domain, its aliases and its DKIM keys from this server."}
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
          <Trash2 className="h-3.5 w-3.5" />
          Delete domain
        </Button>
      </div>
      <ConfirmAction
        open={open}
        onOpenChange={setOpen}
        title={`Delete ${domain.name}?`}
        description="Mail sent to this domain will be refused. You'll still need to remove its records at your DNS provider."
        confirmLabel="Delete domain"
        destructive
        typeToConfirm={domain.name}
        onConfirm={async () => {
          await api.deleteDomain(domain.id);
          toast.success(`${domain.name} was deleted.`);
          refreshHealth();
          navigate("/console/domains", { replace: true });
        }}
      >
        {domain.mailboxCount > 0 && (
          <div className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs leading-relaxed">
            <StatusBadge tone="warn" className="mb-1.5">
              Has dependents
            </StatusBadge>
            <p>
              {domain.mailboxCount} mailbox{domain.mailboxCount === 1 ? " still uses" : "es still use"} this domain. The server will refuse
              until they're deleted —{" "}
              <Link to={`/console/mailboxes?domain=${domain.id}`} className="underline underline-offset-2">
                review them
              </Link>
              .
            </p>
          </div>
        )}
      </ConfirmAction>
    </section>
  );
}
