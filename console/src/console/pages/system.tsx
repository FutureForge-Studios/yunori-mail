import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { Stethoscope } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Card, Chip, KV, SectionLabel, Skeleton } from "@/design/vendor/admin/ui";
import { api, errorMessage } from "@/api/client";
import type { Check, ServiceStatus } from "@/api/types";
import { LoadError, Page, usePageTitle } from "@/components/kit";
import { Meter } from "@/components/meter";
import { CheckList, CheckPill, StatusBadge, worstStatus, type Tone } from "@/components/status";
import { fmtBytes, fmtDate, pct, relativeDays, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";

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
            <div className="mt-3 border-t border-border/50 pt-3">
              <SectionLabel>Certificate</SectionLabel>
              {data.certificate ? (
                <>
                  <KV label="Issuer">{data.certificate.issuer ?? "—"}</KV>
                  <KV label="Expires">
                    {data.certificate.notAfter ? `${fmtDate(data.certificate.notAfter)} (${relativeDays(data.certificate.notAfter)})` : "—"}
                  </KV>
                  <div className="flex items-center justify-between gap-4 py-1.5">
                    <span className="text-xs text-muted-foreground">Served on 465/993</span>
                    {data.certificate.servedMatches === null ? (
                      <StatusBadge tone="muted">Not checked</StatusBadge>
                    ) : data.certificate.servedMatches ? (
                      <StatusBadge tone="ok">Matches</StatusBadge>
                    ) : (
                      <StatusBadge tone="bad">Different certificate</StatusBadge>
                    )}
                  </div>
                  <div className="mt-2 flex flex-wrap gap-1">
                    {data.certificate.hostnames.map((h) => (
                      <Chip key={h}>{h}</Chip>
                    ))}
                  </div>
                </>
              ) : (
                <p className="text-xs text-muted-foreground">No certificate yet — it's issued once the host name resolves to this server.</p>
              )}
            </div>
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
