import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { ArrowLeft, CheckCircle2, RefreshCw, Settings2, Trash2 } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/ui/button";
import { Badge } from "@/ui/badge";
import { Card, CardHeader, SectionHeader } from "@/ui/card";
import { RingMeter, StatCard } from "@/ui/data-viz";
import { EmptyState, Skeleton } from "@/ui/feedback";
import { Field, Select, SwitchRow } from "@/ui/form";
import { PageContainer, PageHeader } from "@/ui/page";
import { Segmented } from "@/ui/segmented";
import { api, errorMessage } from "@/api/client";
import type { DnsRecord, Domain, DomainDnsResponse, Mailbox } from "@/api/types";
import { ConfirmAction } from "@/components/confirm";
import { CopyButton } from "@/components/copy-button";
import { LoadError, Page, usePageTitle } from "@/components/kit";
import { CheckPill, StatusBadge } from "@/components/status";
import { fmtDateTime, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";
import { useCrumb, useShell } from "../shell";
import { DnsProviderCard } from "./domain-dns-provider";

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
  const [filter, setFilter] = useState<"all" | "attention" | "required" | "optional">("all");

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
        <div className="grid grid-cols-2 gap-4 xl:grid-cols-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <Skeleton key={i} className="h-28 rounded-xl" />
          ))}
        </div>
        <Skeleton className="mt-4 h-[28rem] rounded-xl" />
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
  const shown = sortRecords(
    filter === "all" ? dns.records : filter === "attention" ? attention : dns.records.filter((r) => r.required === (filter === "required")),
  );

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

  const optional = dns.records.filter((r) => !r.required);
  const optionalOk = optional.filter((r) => r.status === "pass" || r.status === "info").length;
  const requiredTone = requiredOk === required.length ? "ok" : required.some((r) => r.status === "fail") ? "bad" : "warn";

  return (
    <PageContainer>
      <PageHeader
        eyebrow={
          <Link to="/console/domains" className="inline-flex items-center gap-1 rounded-sm hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60">
            <ArrowLeft aria-hidden className="h-3.5 w-3.5" />
            Domains
          </Link>
        }
        title={domain.name}
        meta={
          <>
            {domain.isPrimary && <Badge>Primary</Badge>}
            {domain.enabled ? <StatusBadge tone="ok">Receiving</StatusBadge> : <StatusBadge tone="muted">Not receiving</StatusBadge>}
            <span className="text-[13px] text-muted-foreground">
              {domain.mailboxCount} mailbox{domain.mailboxCount === 1 ? "" : "es"} · {dns.checkedAt ? `checked ${timeAgo(dns.checkedAt)}` : "not checked yet"}
            </span>
          </>
        }
        actions={
          <>
            <CopyButton value={dns.zoneFile} label="zone file" showText text="Copy zone file" className="h-8" />
            <Button size="sm" onClick={() => void checkNow()} loading={checking}>
              {!checking && <RefreshCw className="h-3.5 w-3.5" />}
              {checking ? "Checking…" : "Check now"}
            </Button>
          </>
        }
      />

      <div className="grid grid-cols-2 gap-3 sm:gap-4 xl:grid-cols-4">
        <StatCard
          label="Required records"
          value={
            <>
              {requiredOk}
              <span className="text-lg text-muted-foreground">/{required.length}</span>
            </>
          }
          hint={requiredOk === required.length ? "All resolve" : `${required.length - requiredOk} not resolving yet`}
          tone={requiredTone === "ok" ? undefined : requiredTone}
        >
          <RingMeter
            value={required.length ? (requiredOk / required.length) * 100 : null}
            tone={requiredTone}
            label="Required records resolving"
            valueText={`${requiredOk} of ${required.length}`}
            size={44}
            stroke={5}
            center={<span />}
            className="hidden sm:inline-grid"
          />
        </StatCard>
        <StatCard
          label="Optional records"
          value={
            <>
              {optionalOk}
              <span className="text-lg text-muted-foreground">/{optional.length}</span>
            </>
          }
          hint="Autoconfig, MTA-STS, SRV, CAA"
        >
          <RingMeter
            value={optional.length ? (optionalOk / optional.length) * 100 : null}
            tone={optionalOk === optional.length ? "ok" : "muted"}
            label="Optional records resolving"
            valueText={`${optionalOk} of ${optional.length}`}
            size={44}
            stroke={5}
            center={<span />}
            className="hidden sm:inline-grid"
          />
        </StatCard>
        <StatCard to={`/console/mailboxes?domain=${domain.id}`} label="Mailboxes" value={domain.mailboxCount} hint={mailboxes.length ? "Open the list" : "None yet"} />
        <StatCard label="Catch-all" value={domain.catchAll ? "On" : "Off"} hint={domain.catchAll ?? "Unknown addresses are refused"} />
      </div>

      <div className="mt-4 grid gap-4 xl:grid-cols-12">
        <div className="min-w-0 xl:col-span-7">
          <DnsProviderCard
            domain={domain}
            onChecked={(fresh) => {
              setData((d) => (d ? { ...d, dns: fresh } : d));
              refreshHealth();
            }}
          />
        </div>
        <div className="min-w-0 xl:col-span-5">
          <SettingsCard domain={domain} mailboxes={mailboxes} onSaved={(d) => setData((x) => (x ? { ...x, domain: d } : x))} />
        </div>
      </div>

      <section className="mt-10" aria-labelledby="dns-title">
        <SectionHeader
          id="dns-title"
          title="DNS records"
          description={
            <>
              Publish these at your DNS provider. “Found” is what public resolvers return right now
              {dns.checkedAt ? ` (as of ${fmtDateTime(dns.checkedAt)})` : ""}; long TXT values are split automatically in the zone file.
            </>
          }
          actions={
            <Segmented
              label="Filter records"
              value={filter}
              onChange={setFilter}
              size="sm"
              options={[
                { value: "all", label: "All", count: dns.records.length },
                { value: "attention", label: "Needs attention", count: attention.length },
                { value: "required", label: "Required", count: required.length },
                { value: "optional", label: "Optional", count: optional.length },
              ]}
            />
          }
        />
        <DnsTable records={shown} />
      </section>

      <DangerZone domain={domain} />
    </PageContainer>
  );
}

const STATUS_ORDER = { fail: 0, warn: 1, unknown: 2, info: 3, pass: 4 } as const;
function sortRecords(records: DnsRecord[]): DnsRecord[] {
  return [...records].sort((a, b) => Number(b.required) - Number(a.required) || STATUS_ORDER[a.status] - STATUS_ORDER[b.status]);
}

// ── DNS table ────────────────────────────────────────────────────────────────

function DnsTable({ records }: { records: DnsRecord[] }) {
  if (records.length === 0) return <EmptyState icon={CheckCircle2} title="Nothing needs attention" hint="Every record resolves as expected." />;
  return (
    <div className="overflow-hidden rounded-xl border border-border bg-card shadow-card">
      <div aria-hidden className="hidden grid-cols-[minmax(15rem,0.9fr)_minmax(0,1.6fr)_7.5rem] gap-6 border-b border-border bg-muted/40 px-5 py-2.5 text-xs font-medium text-muted-foreground md:grid">
        <span>Record</span>
        <span>Value</span>
        <span>Status</span>
      </div>
      <ul className="divide-y divide-border/60" aria-label="DNS records for this domain: what to publish, what resolvers return, and status">
        {records.map((r) => (
          <DnsRow key={r.id} record={r} />
        ))}
      </ul>
    </div>
  );
}

function DnsRow({ record: r }: { record: DnsRecord }) {
  const [expanded, setExpanded] = useState(false);
  const long = r.expected.length > 110;
  const matches = r.actual.length > 0 && r.actual.includes(r.expected);
  const bad = r.status === "fail" || r.status === "warn";
  return (
    <li className={cn("grid gap-3 px-4 py-4 sm:px-5 md:grid-cols-[minmax(15rem,0.9fr)_minmax(0,1.6fr)_7.5rem] md:gap-6", bad && "bg-warning/[0.04]")}>
      <div className="min-w-0">
        <div className="flex items-start gap-2">
          <span className="mt-px w-12 shrink-0 rounded-md border border-border bg-muted/60 py-0.5 text-center font-mono text-2xs font-medium text-foreground/80">{r.type}</span>
          <span className="min-w-0 flex-1 break-all pt-px font-mono text-[12.5px] text-foreground">{r.name}</span>
          <CopyButton value={r.name} label={`name ${r.name}`} className="-mt-0.5 h-6 w-6" />
        </div>
        <p className="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">{r.purpose}</p>
        <div className="mt-2 flex items-center gap-2 md:hidden">
          <CheckPill status={r.status} />
          <span className="text-xs text-muted-foreground">{r.required ? "Required" : "Optional"}</span>
        </div>
      </div>
      <div className="min-w-0 space-y-2">
        <div className="flex items-start gap-2 rounded-lg border border-border/70 bg-muted/30 px-3 py-2">
          <span className="w-14 shrink-0 pt-px text-xs text-muted-foreground">Publish</span>
          <div className="min-w-0 flex-1">
            <code className={cn("block break-all font-mono text-[12.5px] leading-relaxed text-foreground", long && !expanded && "line-clamp-2")}>{r.expected}</code>
            {long && (
              <button
                type="button"
                onClick={() => setExpanded((v) => !v)}
                aria-expanded={expanded}
                className="mt-0.5 rounded-sm text-xs font-medium text-muted-foreground underline underline-offset-2 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
              >
                {expanded ? "Show less" : `Show all ${r.expected.length} characters`}
              </button>
            )}
          </div>
          <CopyButton value={r.expected} label={`${r.type} value for ${r.name}`} className="-my-0.5 h-6 w-6" />
        </div>
        <div className="flex items-start gap-2 px-3">
          <span className="w-14 shrink-0 pt-px text-xs text-muted-foreground">Found</span>
          <div className="min-w-0 flex-1">
            {r.actual.length === 0 ? (
              <span className="text-[13px] text-danger-ink">Nothing published yet</span>
            ) : matches ? (
              <span className="text-[13px] text-muted-foreground">Same as above</span>
            ) : (
              r.actual.map((a, i) => (
                <code key={i} className="block break-all font-mono text-[12.5px] text-foreground/75">
                  {a}
                </code>
              ))
            )}
          </div>
        </div>
        {r.remedy && (
          <p className="rounded-lg bg-warning/10 px-3 py-2 text-[13px] leading-relaxed text-foreground/90">
            <span className="font-medium">What to do: </span>
            {r.remedy}
          </p>
        )}
      </div>
      <div className="hidden md:block">
        <CheckPill status={r.status} />
        <p className="mt-1.5 text-xs text-muted-foreground">{r.required ? "Required" : "Optional"}</p>
      </div>
    </li>
  );
}

// ── Settings ─────────────────────────────────────────────────────────────────

function SettingsCard({ domain, mailboxes, onSaved }: { domain: Domain; mailboxes: Mailbox[]; onSaved: (d: Domain) => void }) {
  const [value, setValue] = useState(domain.catchAll ?? "");
  const [busy, setBusy] = useState<"catch" | "enabled" | null>(null);
  useEffect(() => setValue(domain.catchAll ?? ""), [domain.catchAll]);
  const dirty = value !== (domain.catchAll ?? "");

  async function save() {
    setBusy("catch");
    try {
      const d = await api.updateDomain(domain.id, { catchAll: value || null });
      onSaved(d);
      toast.success(value ? `Mail to unknown addresses at ${domain.name} now goes to ${value}.` : "Catch-all is off.");
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy(null);
    }
  }

  async function toggle(enabled: boolean) {
    setBusy("enabled");
    try {
      onSaved(await api.updateDomain(domain.id, { enabled }));
      toast.success(enabled ? `${domain.name} accepts mail again.` : `${domain.name} now refuses new mail. Existing mail is kept.`);
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Card padded={false} className="h-full" aria-labelledby="dom-settings">
      <CardHeader id="dom-settings" title="Settings" icon={Settings2} divider />
      <div className="divide-y divide-border/70">
        <div className="px-5 py-4">
          <SwitchRow
            id="domain-enabled"
            title={`Accept mail for ${domain.name}`}
            description={
              domain.isPrimary
                ? "The primary domain names this server, so it's always on."
                : "Turning this off refuses new mail for the domain. Mailboxes and their mail stay as they are."
            }
            checked={domain.enabled}
            disabled={busy !== null || domain.isPrimary}
            onCheckedChange={(v) => void toggle(v)}
            stateText
          />
        </div>
        <div className="px-5 py-4">
          <p className="text-sm font-medium text-foreground">Catch-all</p>
          <p className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">
            Mail to an address that doesn't exist is normally refused. A catch-all delivers it to one mailbox instead — handy, but it also collects spam sent
            to guessed addresses.
          </p>
          <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-end">
            <Field label="Deliver unknown addresses to" htmlFor="catch-all" className="min-w-0 flex-1">
              <Select id="catch-all" value={value} onChange={(e) => setValue(e.target.value)}>
                <option value="">Off — refuse them</option>
                {mailboxes.map((m) => (
                  <option key={m.id} value={m.address}>
                    {m.address}
                  </option>
                ))}
              </Select>
            </Field>
            <Button size="sm" onClick={() => void save()} disabled={!dirty} loading={busy === "catch"} className="h-9 shrink-0">
              {busy === "catch" ? "Saving…" : "Save"}
            </Button>
          </div>
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
    <section className="mt-10 rounded-xl border border-destructive/25 bg-destructive/[0.03] p-5" aria-labelledby="danger-title">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 id="danger-title" className="text-sm font-semibold text-foreground">
            Delete {domain.name}
          </h2>
          <p className="mt-0.5 text-[13px] text-muted-foreground">
            {domain.mailboxCount > 0
              ? `Only possible once its ${domain.mailboxCount} mailbox${domain.mailboxCount === 1 ? " is" : "es are"} deleted — mail is never removed silently.`
              : "Removes the domain, its aliases and its DKIM keys from this server."}
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => setOpen(true)} className="text-danger-ink hover:text-danger-ink">
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
          <div className="rounded-lg border border-border bg-muted/40 px-3 py-2.5 text-[13px] leading-relaxed">
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
