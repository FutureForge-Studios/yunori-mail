import { useCallback, useState } from "react";
import { Link } from "react-router-dom";
import { AlertTriangle, ArrowRight, AtSign, CheckCircle2, Globe, HardDrive, Inbox, ListOrdered, Plus, Stethoscope, XCircle } from "lucide-react";
import { api } from "@/api/client";
import type { FirstMailboxResponse, Mailbox } from "@/api/types";
import { CheckList, StatusBadge } from "@/components/status";
import { fmtBytes, fmtNumber, pct } from "@/lib/format";
import { useApi } from "@/lib/use-api";
import { Button } from "@/ui/button";
import { Card, CardBody, CardHeader, KV, KVList } from "@/ui/card";
import { BarList, RingMeter, StatCard, usageTone } from "@/ui/data-viz";
import { EmptyState, LoadError, Skeleton } from "@/ui/feedback";
import { PageContainer, PageHeader, usePageTitle } from "@/ui/page";
import { useSession } from "../session";
import { SIGNAL_TARGET, VERDICT_META, useShell } from "../shell";
import { navLocation } from "../nav";
import { AlertList } from "../alert-list";
import { InstallChecklist } from "./install-checklist";
import { SetupComplete } from "./setup-complete";

const VERDICT_ICON = { ok: CheckCircle2, attention: AlertTriangle, degraded: XCircle, down: XCircle } as const;

/**
 * The status page: one sentence that names the consequence, the numbers, the
 * signals, storage — and the install checklist until setup is complete.
 */
export function OverviewPage() {
  usePageTitle("Overview");
  const { session } = useSession();
  const { refreshHealth } = useShell();
  const { data, error, reload } = useApi("overview", api.overview);
  // Storage is a nice-to-have: the page never waits on it.
  const mailboxes = useApi<Mailbox[]>("overview:mailboxes", () => api.mailboxes().catch(() => []));
  // The end of onboarding stays on screen until dismissed, even after the checklist goes.
  const [created, setCreated] = useState<FirstMailboxResponse | null>(null);
  const [finished, setFinished] = useState(false);

  const onInstallComplete = useCallback(() => {
    setFinished(true);
    void reload();
    void mailboxes.reload();
    refreshHealth();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [reload, refreshHealth]);

  if (error && !data) {
    return (
      <PageContainer>
        <LoadError error={error} onRetry={() => void reload()} what="The overview" />
      </PageContainer>
    );
  }

  if (!data) {
    return (
      <PageContainer>
        <div aria-busy="true">
          <Skeleton className="h-4 w-40" />
          <Skeleton className="mt-3 h-10 w-full max-w-2xl" />
          <div className="mt-8 grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
            {Array.from({ length: 4 }).map((_, i) => (
              <Skeleton key={i} className="h-[7.5rem] rounded-xl" />
            ))}
          </div>
          <div className="mt-4 grid gap-4 xl:grid-cols-3">
            <Skeleton className="h-96 rounded-xl xl:col-span-2" />
            <Skeleton className="h-96 rounded-xl" />
          </div>
        </div>
      </PageContainer>
    );
  }

  const meta = VERDICT_META[data.verdict.level];
  const { server } = session;
  const setupMode = !!data.setup && data.setup.steps.length > 0;
  // Where the worst signal gets fixed (skipped while the checklist is showing the way).
  const worst = data.setup ? undefined : (data.signals.find((s) => s.status === "fail") ?? data.signals.find((s) => s.status === "warn"));
  const fixTo = worst ? SIGNAL_TARGET[worst.id] : undefined;
  const fixLabel = fixTo ? navLocation(fixTo)?.item.label : undefined;
  const passing = data.signals.filter((s) => s.status === "pass").length;
  const signalPct = data.signals.length ? (passing / data.signals.length) * 100 : null;
  const signalTone = data.signals.some((s) => s.status === "fail") ? "bad" : data.signals.some((s) => s.status === "warn") ? "warn" : "ok";

  return (
    <PageContainer>
      <PageHeader
        eyebrow={
          <span className="flex items-center gap-2">
            Overview <span aria-hidden>·</span> <span className="font-mono">{server.hostname}</span>
          </span>
        }
        title={setupMode ? "Setting up your mail server" : data.verdict.sentence}
        blurb={
          setupMode
            ? "Every step is checked for real: DNS against public resolvers, the certificate with Let's Encrypt. You can close this page; setup carries on by itself and picks up the moment something changes."
            : undefined
        }
        meta={
          <>
            <StatusBadge tone={setupMode ? "warn" : meta.tone} icon={setupMode ? AlertTriangle : VERDICT_ICON[data.verdict.level]}>
              {setupMode ? "Setting up" : meta.label}
            </StatusBadge>
            <span className="text-[13px] text-muted-foreground">
              Sending {data.sendingMode === "relay" ? "through a relay" : "directly"} · Yunori {server.version}
              {server.engineVersion ? ` · Stalwart ${server.engineVersion}` : ""}
            </span>
            {fixTo && fixLabel && data.verdict.level !== "ok" && (
              <Link
                to={fixTo}
                className="inline-flex items-center gap-1 rounded-sm text-[13px] font-medium text-foreground underline decoration-border-strong underline-offset-4 transition-colors hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
              >
                Fix it in {fixLabel} <ArrowRight aria-hidden className="h-3.5 w-3.5" />
              </Link>
            )}
          </>
        }
        actions={
          !setupMode && (
            <>
              <Button asChild variant="outline" size="sm">
                <Link to="/console/system?preflight=1">
                  <Stethoscope className="h-4 w-4" />
                  Run preflight
                </Link>
              </Button>
              <Button asChild size="sm">
                <Link to="/console/mailboxes?new=1">
                  <Plus className="h-4 w-4" />
                  New mailbox
                </Link>
              </Button>
            </>
          )
        }
      />

      {(created || finished) && (
        <SetupComplete
          created={created}
          finished={finished || !data.setup}
          onDismiss={() => {
            setCreated(null);
            setFinished(false);
          }}
        />
      )}

      {data.setup && data.setup.steps.length > 0 && (
        <InstallChecklist initial={data.setup} onComplete={onInstallComplete} onMailboxCreated={setCreated} />
      )}

      {data.alerts?.length > 0 && (
        <Card padded={false} className="mb-4 border-warning/30" aria-labelledby="ov-alerts">
          <CardHeader
            id="ov-alerts"
            title={`${data.alerts.length} alert${data.alerts.length === 1 ? "" : "s"} firing`}
            icon={AlertTriangle}
            divider
            actions={
              <Button asChild variant="ghost" size="sm">
                <Link to="/console/alerts">
                  Alert settings
                  <ArrowRight className="h-3.5 w-3.5" />
                </Link>
              </Button>
            }
          />
          <CardBody className="pt-1">
            <AlertList alerts={data.alerts} compact />
          </CardBody>
        </Card>
      )}

      {setupMode && <h2 className="mb-3 mt-8 text-sm font-semibold text-foreground">This server right now</h2>}
      <div className="grid grid-cols-2 gap-3 sm:gap-4 xl:grid-cols-4">
        <StatCard to="/console/domains" icon={Globe} label="Domains" value={fmtNumber(data.stats.domains)} hint={server.primaryDomain ? `Primary: ${server.primaryDomain}` : "None yet"} />
        <StatCard to="/console/mailboxes" icon={Inbox} label="Mailboxes" value={fmtNumber(data.stats.mailboxes)} hint="Across all domains" />
        <StatCard to="/console/aliases" icon={AtSign} label="Aliases" value={fmtNumber(data.stats.aliases)} hint="Extra addresses" />
        <StatCard
          to="/console/queue"
          icon={ListOrdered}
          label="Queued"
          value={fmtNumber(data.stats.queued)}
          hint={data.stats.queued === 0 ? "Nothing waiting" : "Waiting to be delivered"}
          tone={data.stats.queued > 0 ? "warn" : undefined}
        />
      </div>

      <div className="mt-4 grid gap-4 xl:grid-cols-12">
        <Card padded={false} className="xl:col-span-8" aria-labelledby="ov-signals">
          <CardHeader
            id="ov-signals"
            title="Signals"
            description="What decides whether mail flows: DNS, reverse DNS, TLS, port 25, blocklists and the engine. Worst first."
            divider
            actions={
              <div className="flex items-center gap-3">
                <div className="text-right">
                  <p className="display-number text-2xl leading-none">
                    {passing}
                    <span className="text-base text-muted-foreground">/{data.signals.length}</span>
                  </p>
                  <p className="mt-1 text-xs text-muted-foreground">passing</p>
                </div>
                <RingMeter
                  value={signalPct}
                  tone={signalTone}
                  label="Signals passing"
                  valueText={`${passing} of ${data.signals.length} passing`}
                  size={48}
                  stroke={5}
                  center={<span />}
                />
              </div>
            }
          />
          <CardBody className="pb-2">
            <CheckList checks={data.signals} compact />
          </CardBody>
        </Card>

        <div className="grid content-start gap-4 md:grid-cols-2 xl:col-span-4 xl:grid-cols-1">
          <StorageCard mailboxes={mailboxes.data} />
          <Card padded={false} aria-labelledby="ov-server">
            <CardHeader id="ov-server" title="This server" divider />
            <CardBody className="pt-1">
              <KVList>
                <KV label="Hostname">
                  <span className="font-mono text-[12.5px]">{server.hostname}</span>
                </KV>
                <KV label="Primary domain">{server.primaryDomain ?? "—"}</KV>
                <KV label="Sending">{data.sendingMode === "relay" ? "Relay (smarthost)" : "Direct"}</KV>
                <KV label="Yunori">{server.version}</KV>
                <KV label="Mail engine">{server.engineVersion ? `Stalwart ${server.engineVersion}` : "—"}</KV>
                <KV label="Setup">{server.installComplete ? "Complete" : "In progress"}</KV>
              </KVList>
              <div className="mt-3 grid gap-1 border-t border-border/70 pt-3">
                <QuickLink to="/console/mailboxes?new=1">Create a mailbox</QuickLink>
                <QuickLink to="/console/domains?new=1">Add a domain</QuickLink>
                <QuickLink to="/console/aliases?new=1">Add an alias</QuickLink>
              </div>
            </CardBody>
          </Card>
        </div>
      </div>
    </PageContainer>
  );
}

/** Storage across mailboxes: total used, and the fullest few. */
function StorageCard({ mailboxes }: { mailboxes: Mailbox[] | undefined }) {
  const used = (mailboxes ?? []).reduce((n, m) => n + (m.usedBytes ?? 0), 0);
  const limited = (mailboxes ?? []).filter((m) => m.quotaBytes);
  const quota = limited.reduce((n, m) => n + (m.quotaBytes ?? 0), 0);
  const top = [...(mailboxes ?? [])].sort((a, b) => (b.usedBytes ?? 0) - (a.usedBytes ?? 0)).slice(0, 4);
  const p = pct(used, quota);
  return (
    <Card padded={false} aria-labelledby="ov-storage">
      <CardHeader
        id="ov-storage"
        title="Storage"
        icon={HardDrive}
        actions={
          <Link to="/console/mailboxes" className="rounded-sm text-[13px] text-muted-foreground underline-offset-4 hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60">
            All mailboxes
          </Link>
        }
      />
      <CardBody>
        {!mailboxes ? (
          <Skeleton className="h-32" />
        ) : mailboxes.length === 0 ? (
          <EmptyState compact title="No mailboxes yet" hint="Storage shows up here once mail arrives." />
        ) : (
          <>
            <div className="mb-5 flex items-center gap-4">
              <RingMeter value={p} label="Storage used across mailboxes with a limit" valueText={`${fmtBytes(used)} of ${fmtBytes(quota)} allotted`} size={64} stroke={6} />
              <div className="min-w-0">
                <p className="display-number text-2xl leading-none">{fmtBytes(used)}</p>
                <p className="mt-1.5 text-xs text-muted-foreground">
                  used of {fmtBytes(quota)} allotted to {limited.length} mailbox{limited.length === 1 ? "" : "es"}
                </p>
              </div>
            </div>
            <BarList
              label="Largest mailboxes"
              items={top.map((m) => {
                const mp = pct(m.usedBytes, m.quotaBytes);
                return {
                  key: m.id,
                  label: m.address,
                  value: m.usedBytes ?? 0,
                  display: m.quotaBytes ? `${fmtBytes(m.usedBytes ?? 0)} · ${Math.round(mp ?? 0)}%` : fmtBytes(m.usedBytes ?? 0),
                  tone: usageTone(mp),
                };
              })}
            />
          </>
        )}
      </CardBody>
    </Card>
  );
}

function QuickLink({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link
      to={to}
      className="group flex items-center justify-between rounded-lg px-2 py-1.5 text-[13px] text-foreground/80 transition-colors -mx-2 hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
    >
      {children}
      <ArrowRight aria-hidden className="h-3.5 w-3.5 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
    </Link>
  );
}
