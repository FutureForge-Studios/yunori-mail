import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { ArrowUpRight, Cpu, Lock, Package, RefreshCw, Server, Stethoscope } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/ui/button";
import { Card, CardBody, CardHeader, KV, KVList } from "@/ui/card";
import { RingMeter } from "@/ui/data-viz";
import { Skeleton } from "@/ui/feedback";
import { api, errorMessage } from "@/api/client";
import type { Check, ServiceStatus, UpdateStatus } from "@/api/types";
import { CopyButton } from "@/components/copy-button";
import { LoadError, Page, usePageTitle } from "@/components/kit";
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
        <Card padded={false} className="mb-4 animate-rise-in motion-reduce:animate-none">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/70 px-5 py-4">
            <h2 ref={resultsRef} tabIndex={-1} className="flex items-center gap-2 text-[15px] font-semibold focus:outline-none">
              <Stethoscope aria-hidden className="h-4 w-4 text-icon" />
              Preflight results
            </h2>
            <div className="flex items-center gap-2">
              <CheckPill
                status={worstStatus(preflight.checks.map((c) => c.status))}
                label={`${preflight.checks.filter((c) => c.status === "pass").length} of ${preflight.checks.length} passed`}
              />
              <span className="text-xs text-muted-foreground">ran {timeAgo(preflight.at)}</span>
            </div>
          </div>
          <CardBody className="pb-2">
            <CheckList checks={preflight.checks} compact />
          </CardBody>
        </Card>
      )}

      {error && !data ? (
        <LoadError error={error} onRetry={() => void reload()} what="System status" />
      ) : !data ? (
        <div className="grid gap-4 lg:grid-cols-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={i} className="h-64 rounded-xl" />
          ))}
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-12">
          <Card padded={false} className="md:col-span-2 xl:col-span-7" aria-labelledby="sys-res">
            <CardHeader id="sys-res" title="Resources" icon={Cpu} />
            <CardBody>
              <div className="grid grid-cols-2 gap-4 sm:grid-cols-3">
                <Resource label="Memory" used={data.resources.memUsedBytes} total={data.resources.memTotalBytes} />
                <Resource
                  label="Disk"
                  used={data.resources.diskTotalBytes - data.resources.diskFreeBytes}
                  total={data.resources.diskTotalBytes}
                  extra={`${fmtBytes(data.resources.diskFreeBytes)} free`}
                />
                <div className="col-span-2 flex flex-col items-center justify-center rounded-xl border border-border bg-muted/30 px-3 py-4 text-center sm:col-span-1">
                  <span className="display-number text-[34px] leading-none">{data.resources.load1.toFixed(2)}</span>
                  <span className="mt-2 text-[13px] font-medium text-foreground">Load</span>
                  <span className="text-xs text-muted-foreground">1-minute average</span>
                </div>
              </div>
            </CardBody>
          </Card>

          <Card padded={false} className="md:col-span-2 xl:col-span-5" aria-labelledby="sys-svc">
            <CardHeader id="sys-svc" title="Services" icon={Server} divider />
            <CardBody className="pb-2">
            <ul className="divide-y divide-border/60">
              {data.services.map((s) => {
                const m = SERVICE_TONE[s.status];
                return (
                  <li key={s.name} className="py-3">
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-sm font-medium text-foreground">{SERVICE_LABEL[s.name] ?? s.name}</span>
                      <StatusBadge tone={m.tone}>{m.label}</StatusBadge>
                    </div>
                    {s.detail && <p className="mt-0.5 text-[13px] text-muted-foreground">{s.detail}</p>}
                  </li>
                );
              })}
            </ul>
            </CardBody>
          </Card>

          <Card padded={false} className="md:col-span-2 xl:col-span-4" aria-labelledby="sys-ver">
            <CardHeader id="sys-ver" title="Versions" icon={Package} divider />
            <CardBody className="pt-1">
              <KVList>
                <KV label="Yunori Mail">{data.versions.yunori}</KV>
                <KV label="Stalwart">{data.versions.engine ?? "Not installed"}</KV>
              </KVList>
              <Updates initial={data.update} />
            </CardBody>
          </Card>

          <Card padded={false} className="md:col-span-2 xl:col-span-8" aria-labelledby="sys-cert">
            <CardHeader id="sys-cert" title="Certificate" icon={Lock} description="Automatic HTTPS for the console, the portal and the mail ports." />
            <CardBody>
              <CertificatePanel />
            </CardBody>
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
    <div className="flex flex-col items-center rounded-xl border border-border bg-muted/30 px-3 py-4 text-center">
      <RingMeter value={p} label={`${label} used`} valueText={`${text}${extra ? `, ${extra}` : ""}`} size={84} stroke={8} tone={p !== null && p >= 90 ? "warn" : "info"} />
      <span className="mt-3 text-[13px] font-medium text-foreground">{label}</span>
      <span className="text-xs tabular-nums text-muted-foreground">{text}</span>
      {extra && <span className="text-xs tabular-nums text-muted-foreground">{extra}</span>}
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
    <div className="mt-3 border-t border-border/70 pt-3">
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
          className="h-7 px-2 text-xs"
        >
          <RefreshCw aria-hidden className={cn("h-3.5 w-3.5", busy && "animate-spin motion-reduce:animate-none")} />
          {busy ? "Checking…" : "Check now"}
        </Button>
      </div>
      {u.available && (
        <>
          <p className="mt-2 text-[13px] leading-relaxed text-foreground/85">
            {u.publishedAt && <>Released {timeAgo(u.publishedAt)}. </>}
            Run this on the server; your settings and mail stay as they are.
          </p>
          <div className="mt-2 flex items-center gap-1 rounded-lg border border-border bg-muted/40 py-1 pl-3 pr-1">
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
      <p className="mt-2 text-xs text-muted-foreground">
        {u.problem
          ? `Last check failed: ${u.problem}`
          : u.checkedAt
            ? `Checked ${timeAgo(u.checkedAt)} against yunori.email.`
            : "Checked once a day against yunori.email."}
      </p>
    </div>
  );
}
