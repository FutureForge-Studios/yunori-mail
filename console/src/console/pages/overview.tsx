import { useCallback, useState } from "react";
import { Link } from "react-router-dom";
import { AlertTriangle, ArrowRight, CheckCircle2, XCircle } from "lucide-react";
import { Card, KV, SectionLabel, Skeleton, StatCard } from "@/design/vendor/admin/ui";
import { api } from "@/api/client";
import type { CreateMailboxResponse } from "@/api/types";
import { LoadError, usePageTitle } from "@/components/kit";
import { CheckList, StatusBadge } from "@/components/status";
import { fmtNumber } from "@/lib/format";
import { useApi } from "@/lib/use-api";
import { useSession } from "../session";
import { SIGNAL_TARGET, VERDICT_META, useShell } from "../shell";
import { navLocation } from "../nav";
import { InstallChecklist } from "./install-checklist";
import { SetupComplete } from "./setup-complete";

const VERDICT_ICON = { ok: CheckCircle2, attention: AlertTriangle, degraded: XCircle, down: XCircle } as const;

/**
 * The status page: one sentence that names the consequence, the numbers, the
 * signals — and the install checklist until setup is complete.
 */
export function OverviewPage() {
  usePageTitle("Overview");
  const { session } = useSession();
  const { refreshHealth } = useShell();
  const { data, error, reload } = useApi("overview", api.overview);
  // The end of onboarding stays on screen until dismissed, even after the checklist goes.
  const [created, setCreated] = useState<CreateMailboxResponse | null>(null);
  const [finished, setFinished] = useState(false);

  const onInstallComplete = useCallback(() => {
    setFinished(true);
    void reload();
    refreshHealth();
  }, [reload, refreshHealth]);

  if (error && !data) {
    return (
      <div className="mx-auto max-w-6xl px-5 py-8 sm:px-8 sm:py-10">
        <LoadError error={error} onRetry={() => void reload()} what="The overview" />
      </div>
    );
  }

  if (!data) {
    return (
      <div className="mx-auto max-w-6xl px-5 py-8 sm:px-8 sm:py-10" aria-busy="true">
        <Skeleton className="mb-3 h-3 w-40" />
        <Skeleton className="mb-10 h-10 w-full max-w-2xl" />
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <Skeleton key={i} className="h-24" />
          ))}
        </div>
        <Skeleton className="mt-4 h-72" />
      </div>
    );
  }

  const meta = VERDICT_META[data.verdict.level];
  const { server } = session;
  // Where the worst signal gets fixed (skipped while the checklist is showing the way).
  const worst = data.setup ? undefined : data.signals.find((s) => s.status === "fail") ?? data.signals.find((s) => s.status === "warn");
  const fixTo = worst ? SIGNAL_TARGET[worst.id] : undefined;
  const fixLabel = fixTo ? navLocation(fixTo)?.item.label : undefined;

  return (
    <div className="mx-auto max-w-6xl px-5 py-8 sm:px-8 sm:py-10">
      <header className="mb-8">
        <p className="mb-3 text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
          Overview · <span className="normal-case tracking-normal font-mono">{server.hostname}</span>
        </p>
        <h1 className="max-w-3xl font-serif text-[28px] leading-[1.15] tracking-tight sm:text-[34px]">{data.verdict.sentence}</h1>
        <div className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2">
          <StatusBadge tone={meta.tone} icon={VERDICT_ICON[data.verdict.level]}>
            {meta.label}
          </StatusBadge>
          <span className="text-xs text-muted-foreground">
            Sending {data.sendingMode === "relay" ? "through a relay" : "directly"} · Yunori {server.version}
            {server.engineVersion ? ` · Stalwart ${server.engineVersion}` : ""}
          </span>
          {fixTo && fixLabel && data.verdict.level !== "ok" && (
            <Link
              to={fixTo}
              className="inline-flex items-center gap-1 rounded-sm text-xs text-foreground/85 underline underline-offset-2 transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              Fix it in {fixLabel} <ArrowRight aria-hidden className="h-3 w-3" />
            </Link>
          )}
        </div>
      </header>

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

      <div className="grid grid-cols-2 gap-4 xl:grid-cols-4">
        <StatLink to="/console/domains" label="Domains" value={data.stats.domains} hint={server.primaryDomain ? `Primary: ${server.primaryDomain}` : undefined} />
        <StatLink to="/console/mailboxes" label="Mailboxes" value={data.stats.mailboxes} hint="Across all domains" />
        <StatLink to="/console/aliases" label="Aliases" value={data.stats.aliases} hint="Extra addresses" />
        <StatLink
          to="/console/deliverability"
          label="Queued"
          value={data.stats.queued}
          hint={data.stats.queued === 0 ? "Nothing waiting" : "Waiting to be delivered"}
        />
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <div className="flex items-baseline justify-between gap-3">
            <SectionLabel>Signals</SectionLabel>
            <span className="text-[11px] text-muted-foreground">
              {data.signals.filter((s) => s.status === "pass").length} of {data.signals.length} passing
            </span>
          </div>
          <CheckList checks={data.signals} compact />
        </Card>

        <Card>
          <SectionLabel>This server</SectionLabel>
          <KV label="Hostname">
            <span className="font-mono">{server.hostname}</span>
          </KV>
          <KV label="Primary domain">{server.primaryDomain ?? "—"}</KV>
          <KV label="Sending">{data.sendingMode === "relay" ? "Relay (smarthost)" : "Direct"}</KV>
          <KV label="Yunori">{server.version}</KV>
          <KV label="Mail engine">{server.engineVersion ? `Stalwart ${server.engineVersion}` : "—"}</KV>
          <KV label="Setup">{server.installComplete ? "Complete" : "In progress"}</KV>
          <div className="mt-3 space-y-1.5 border-t border-border/50 pt-3">
            <QuickLink to="/console/mailboxes?new=1">Create a mailbox</QuickLink>
            <QuickLink to="/console/domains?new=1">Add a domain</QuickLink>
            <QuickLink to="/console/system?preflight=1">Run preflight checks</QuickLink>
          </div>
        </Card>
      </div>
    </div>
  );
}

function StatLink({ to, label, value, hint }: { to: string; label: string; value: number; hint?: string }) {
  return (
    <Link to={to} className="block rounded-xl transition-colors hover:[&>div]:border-foreground/20 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
      <StatCard label={label} value={fmtNumber(value)} hint={hint} />
    </Link>
  );
}

function QuickLink({ to, children }: { to: string; children: React.ReactNode }) {
  return (
    <Link
      to={to}
      className="flex items-center justify-between rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
    >
      {children}
      <ArrowRight aria-hidden className="h-3 w-3" />
    </Link>
  );
}
