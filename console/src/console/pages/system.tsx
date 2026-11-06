import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { ArrowUpRight, RefreshCw, Stethoscope } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Card, KV, SectionLabel, Skeleton } from "@/design/vendor/admin/ui";
import { api, errorMessage } from "@/api/client";
import type { Check, ServiceStatus, UpdateStatus } from "@/api/types";
import { CopyButton } from "@/components/copy-button";
import { LoadError, Page, usePageTitle } from "@/components/kit";
import { Meter } from "@/components/meter";
import { CheckList, CheckPill, StatusBadge, worstStatus, type Tone } from "@/components/status";
import { fmtBytes, pct, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";
import { CertificatePanel } from "../certificate";

const SERVICE_LABEL: Record<string, string> = {
  stalwart: "Mail engine (Stalwart)",
  "yunori-mail": "Yunori control plane",
  unbound: "DNS resolver (Unbound)",
};
const SERVICE_TONE: Record<ServiceStatus["status"], { tone: Tone; label: string }> = {
  running: { tone: "ok", label: "Running" },
  stopped: { tone: "muted", label: "Stopped" },
  failed: { tone: "bad", label: "Failed" },
  unknown: { tone: "muted", label: "Unknown" },
};

/** Services, versions, resources, certificate — and an on-demand preflight. */
export default function SystemPage() {
  usePageTitle("System");
  const [params, setParams] = useSearchParams();
  const { data, error, reload } = useApi("system", api.system);
  const [preflight, setPreflight] = useState<{ checks: Check[]; at: string } | null>(null);
  const [running, setRunning] = useState(false);
  const resultsRef = useRef<HTMLHeadingElement>(null);

  async function runPreflight() {
    setRunning(true);
    try {
      const checks = await api.preflight();
      setPreflight({ checks, at: new Date().toISOString() });
      requestAnimationFrame(() => resultsRef.current?.focus());
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setRunning(false);
    }
  }

  const auto = params.get("preflight") === "1";
  useEffect(() => {
    if (!auto) return;
    setParams(
      (p) => {
        const n = new URLSearchParams(p);
        n.delete("preflight");
        return n;
      },
      { replace: true },
    );
    void runPreflight();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auto]);

  const actions = (
    <Button size="sm" variant="outline" onClick={() => void runPreflight()} disabled={running}>
      <Stethoscope className={cn("h-4 w-4", running && "animate-pulse motion-reduce:animate-none")} />
      {running ? "Running preflight…" : "Run preflight"}
    </Button>
  );

  return (
    <Page title="System" blurb="Services, versions and resources. Logs and diagnostics never include mail contents." actions={actions}>
      {preflight && (
        <Card className="mb-4">
          <div className="flex flex-wrap items-baseline justify-between gap-2">
            <h2 ref={resultsRef} tabIndex={-1} className="text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground focus:outline-none">
              Preflight results
            </h2>
            <div className="flex items-center gap-2">
              <CheckPill
                status={worstStatus(preflight.checks.map((c) => c.status))}
                label={`${preflight.checks.filter((c) => c.status === "pass").length} of ${preflight.checks.length} passed`}
              />
              <span className="text-[11px] text-muted-foreground">ran {timeAgo(preflight.at)}</span>
            </div>
          </div>
          <CheckList checks={preflight.checks} compact />
        </Card>
      )}

      {error && !data ? (
        <LoadError error={error} onRetry={() => void reload()} what="System status" />
      ) : !data ? (
        <div className="grid gap-4 lg:grid-cols-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={i} className="h-60" />
          ))}
        </div>
      ) : (
        <div className="grid gap-4 lg:grid-cols-3">
          <Card>
            <SectionLabel>Services</SectionLabel>
            <ul className="divide-y divide-border/50">
              {data.services.map((s) => {
                const m = SERVICE_TONE[s.status];
                return (
                  <li key={s.name} className="py-2.5">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-sm text-foreground/90">{SERVICE_LABEL[s.name] ?? s.name}</span>
                      <StatusBadge tone={m.tone}>{m.label}</StatusBadge>
                    </div>
                    {s.detail && <p className="mt-0.5 text-xs text-muted-foreground">{s.detail}</p>}
                  </li>
                );
              })}
            </ul>
          </Card>

          <Card>
            <SectionLabel>Resources</SectionLabel>
            <Resource
              label="Memory"
              used={data.resources.memUsedBytes}
              total={data.resources.memTotalBytes}
            />
            <Resource
              label="Disk"
              used={data.resources.diskTotalBytes - data.resources.diskFreeBytes}
              total={data.resources.diskTotalBytes}
              extra={`${fmtBytes(data.resources.diskFreeBytes)} free`}
            />
            <div className="mt-4 flex items-baseline justify-between">
              <span className="text-xs text-muted-foreground">Load (1 min)</span>
              <span className="font-serif text-[22px] leading-none tabular-nums">{data.resources.load1.toFixed(2)}</span>
            </div>
          </Card>

          <Card>
            <SectionLabel>Versions</SectionLabel>
            <KV label="Yunori Mail">{data.versions.yunori}</KV>
            <KV label="Stalwart">{data.versions.engine ?? "Not installed"}</KV>
            <Updates initial={data.update} />
          </Card>

          <Card className="lg:col-span-3">
            <SectionLabel>Certificate</SectionLabel>
            <CertificatePanel />
          </Card>
        </div>
      )}
    </Page>
  );
}

function Resource({ label, used, total, extra }: { label: string; used: number; total: number; extra?: string }) {
  const p = pct(used, total);
  const text = `${fmtBytes(used)} of ${fmtBytes(total)}`;
  return (
    <div className="mb-4">
      <div className="mb-1.5 flex items-baseline justify-between gap-2">
        <span className="text-xs text-muted-foreground">{label}</span>
        <span className="text-xs tabular-nums text-foreground/85">
          {text}
          {p !== null && <span className="text-muted-foreground"> · {Math.round(p)}%</span>}
        </span>
      </div>
      <Meter value={p} label={`${label} used`} valueText={`${text}${extra ? `, ${extra}` : ""}`} />
      {extra && <p className="mt-1 text-[11px] text-muted-foreground">{extra}</p>}
    </div>
  );
}

const UPDATE_COMMAND = "sudo yunori-mail update";

/**
 * Whether a newer release is out, from the daily check against
 * yunori.email/server/latest.json. Nothing is installed from here: the
 * command does that on the server, verified and with an engine rollback.
 */
function Updates({ initial }: { initial: UpdateStatus }) {
  const [u, setU] = useState(initial);
  const [busy, setBusy] = useState(false);
  useEffect(() => setU(initial), [initial]);

  async function checkNow() {
    setBusy(true);
    try {
      const next = await api.checkForUpdate();
      setU(next);
      if (next.problem) toast.error(`Couldn't check for updates: ${next.problem}`);
      else toast.success(next.available ? `Yunori Mail ${next.latest} is available.` : "You're on the newest release.");
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-3 border-t border-border/50 pt-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        {u.available ? (
          <StatusBadge tone="warn">{u.latest} is available</StatusBadge>
        ) : u.latest ? (
          <StatusBadge tone="ok">Up to date</StatusBadge>
        ) : (
          <StatusBadge tone="muted">Not checked yet</StatusBadge>
        )}
        <Button
          variant="ghost"
          size="sm"
          onClick={() => void checkNow()}
          disabled={busy}
          aria-label={busy ? "Checking for updates" : "Check now for updates"}
          className="h-7 px-2 text-xs text-muted-foreground"
        >
          <RefreshCw aria-hidden className={cn("h-3.5 w-3.5", busy && "animate-spin motion-reduce:animate-none")} />
          {busy ? "Checking…" : "Check now"}
        </Button>
      </div>
      {u.available && (
        <>
          <p className="mt-2 text-xs leading-relaxed text-foreground/85">
            {u.publishedAt && <>Released {timeAgo(u.publishedAt)}. </>}
            Run this on the server; your settings and mail stay as they are.
          </p>
          <div className="mt-2 flex items-center gap-1 rounded-md border border-border/60 bg-muted/30 py-1 pl-2.5 pr-1">
            <code className="min-w-0 flex-1 truncate font-mono text-xs">{UPDATE_COMMAND}</code>
            <CopyButton value={UPDATE_COMMAND} label="the update command" />
          </div>
          {u.notesUrl && (
            <a
              href={u.notesUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="mt-2 inline-flex items-center gap-1 rounded-sm text-xs text-foreground underline underline-offset-4 hover:no-underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              What changed
              <ArrowUpRight aria-hidden className="h-3 w-3" />
            </a>
          )}
        </>
      )}
      <p className="mt-2 text-[11px] text-muted-foreground">
        {u.problem
          ? `Last check failed: ${u.problem}`
          : u.checkedAt
            ? `Checked ${timeAgo(u.checkedAt)} against yunori.email.`
            : "Checked once a day against yunori.email."}
      </p>
    </div>
  );
}
