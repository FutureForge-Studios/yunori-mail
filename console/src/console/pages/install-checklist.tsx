import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import {
  ArrowRight,
  CheckCircle2,
  ChevronRight,
  Circle,
  Hourglass,
  Loader2,
  MinusCircle,
  RefreshCw,
  XCircle,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Card, SectionLabel } from "@/design/vendor/admin/ui";
import { api, errorMessage } from "@/api/client";
import type { DnsRecord, InstallState, InstallStep, StepStatus } from "@/api/types";
import { CopyButton } from "@/components/copy-button";
import { toneText, type Tone } from "@/components/status";
import { fmtDateTime, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";

const STEP_META: Record<StepStatus, { label: string; icon: LucideIcon; tone: Tone; spin?: boolean }> = {
  done: { label: "Done", icon: CheckCircle2, tone: "ok" },
  running: { label: "Running", icon: Loader2, tone: "muted", spin: true },
  waiting: { label: "Waiting for you", icon: Hourglass, tone: "warn" },
  failed: { label: "Failed", icon: XCircle, tone: "bad" },
  pending: { label: "Not started", icon: Circle, tone: "muted" },
  skipped: { label: "Skipped", icon: MinusCircle, tone: "muted" },
};

const isActive = (s: InstallState) => s.steps.some((x) => x.status === "running" || x.status === "waiting");
const isFinished = (s: InstallState) => s.steps.length > 0 && s.steps.every((x) => x.status === "done" || x.status === "skipped");

/**
 * The installer's checklist, live. Polls GET /install every 3 s while a step
 * is running or waiting on the user; the waiting step is expanded with exactly
 * what to do (for DNS: the records, copyable, plus "Check now").
 */
export function InstallChecklist({ initial, onComplete }: { initial: InstallState; onComplete: () => void }) {
  const [state, setState] = useState<InstallState>(initial);
  const completed = useRef(false);

  const poll = useCallback(async () => {
    try {
      const next = await api.install();
      if (next.steps.length) setState(next);
      if ((next.steps.length === 0 || isFinished(next)) && !completed.current) {
        completed.current = true;
        toast.success("Setup is complete — your server is ready for mail.");
        onComplete();
      }
    } catch {
      /* keep the last state; the next tick retries */
    }
  }, [onComplete]);

  const active = isActive(state);
  useEffect(() => {
    if (!active) return;
    const t = window.setInterval(() => {
      if (document.visibilityState === "visible") void poll();
    }, 3000);
    return () => window.clearInterval(t);
  }, [active, poll]);

  const doneCount = state.steps.filter((s) => s.status === "done" || s.status === "skipped").length;
  const current = state.steps.find((s) => s.status === "waiting" || s.status === "running" || s.status === "failed");

  return (
    <Card className="mb-4 p-0">
      <div className="flex flex-wrap items-end justify-between gap-3 border-b border-border/50 px-5 pb-4 pt-5">
        <div>
          <SectionLabel>Setup checklist</SectionLabel>
          <p className="-mt-1.5 text-sm text-foreground/90">
            {doneCount} of {state.steps.length} steps done
            {current ? <span className="text-muted-foreground"> · now: {current.title}</span> : ""}
          </p>
        </div>
        {active && (
          <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground" aria-live="off">
            <span aria-hidden className="relative flex h-1.5 w-1.5">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-foreground/30 motion-reduce:animate-none" />
              <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-foreground/50" />
            </span>
            Live — refreshes every few seconds
          </p>
        )}
      </div>

      <ol className="px-5 py-2" aria-label="Install steps">
        {state.steps.map((step, i) => (
          <StepRow key={step.id} step={step} index={i + 1} onRecheck={poll} />
        ))}
      </ol>

      <details className="group border-t border-border/50 px-5 py-3">
        <summary className="flex cursor-pointer list-none items-center gap-1.5 rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
          <ChevronRight aria-hidden className="h-3.5 w-3.5 transition-transform group-open:rotate-90 motion-reduce:transition-none" />
          Install log
          <span className="tabular-nums">({state.logs.length} lines)</span>
        </summary>
        <div className="mt-3 max-h-72 overflow-auto rounded-lg border border-border/60 bg-muted/30 p-3" tabIndex={0} aria-label="Install log">
          <ul className="space-y-1 font-mono text-[11px] leading-relaxed">
            {state.logs.map((l, i) => (
              <li key={i} className="flex gap-3">
                <time dateTime={l.at} className="shrink-0 text-muted-foreground" title={fmtDateTime(l.at)}>
                  {new Date(l.at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" })}
                </time>
                <span className={cn("w-10 shrink-0 uppercase", l.level === "info" ? "text-muted-foreground" : toneText(l.level === "warn" ? "warn" : "bad"))}>
                  {l.level}
                </span>
                {l.step && <span className="hidden shrink-0 text-muted-foreground sm:inline">{l.step}</span>}
                <span className="min-w-0 break-words text-foreground/85">{l.message}</span>
              </li>
            ))}
          </ul>
        </div>
      </details>
    </Card>
  );
}

function StepRow({ step, index, onRecheck }: { step: InstallStep; index: number; onRecheck: () => Promise<void> }) {
  const m = STEP_META[step.status];
  const expanded = step.status === "waiting" || step.status === "failed";
  const done = step.status === "done" || step.status === "skipped";
  return (
    <li className={done ? "py-1.5" : "py-2.5"} aria-current={expanded || step.status === "running" ? "step" : undefined}>
      <div className="flex items-start gap-3">
        <m.icon
          aria-hidden
          strokeWidth={1.75}
          className={cn("mt-0.5 h-4 w-4 shrink-0", toneText(m.tone), m.spin && "animate-spin motion-reduce:animate-none", step.status === "pending" && "opacity-50")}
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline justify-between gap-x-3">
            <p className={cn("min-w-0 text-sm", step.status === "pending" ? "text-muted-foreground" : "text-foreground/90", done && "truncate")}>
              <span className="sr-only">Step {index}: </span>
              {step.title}
              {/* Finished steps stay one quiet line; the detail is still there for anyone who looks. */}
              {done && step.detail && <span className="ml-2 hidden text-xs text-muted-foreground md:inline">{step.detail}</span>}
            </p>
            <span className={cn("shrink-0 text-[11px]", done ? "text-muted-foreground" : toneText(m.tone), (expanded || step.status === "running") && "font-medium")}>
              {m.label}
              {done && step.finishedAt && <> · {timeAgo(step.finishedAt)}</>}
            </span>
          </div>
          {step.detail && !expanded && !done && <p className="mt-0.5 text-xs text-muted-foreground">{step.detail}</p>}
        </div>
      </div>
      {expanded && (
        <div className="ml-7 mt-2.5 rounded-lg border border-border bg-muted/30 p-4">
          {step.detail && <p className="text-sm leading-relaxed text-foreground">{step.detail}</p>}
          {step.id === "dns" && step.status === "waiting" && <DnsTodo onRecheck={onRecheck} />}
          {step.id === "ptr" && step.status === "waiting" && (
            <p className="mt-2 text-xs text-muted-foreground">
              Reverse DNS is set in your VPS provider's control panel (often under “Networking” or “rDNS”), not at your DNS host.
            </p>
          )}
        </div>
      )}
    </li>
  );
}

/** The required records still missing on the primary domain, copyable, with a re-check. */
function DnsTodo({ onRecheck }: { onRecheck: () => Promise<void> }) {
  const [domainId, setDomainId] = useState<string | null>(null);
  const [records, setRecords] = useState<DnsRecord[] | null>(null);
  const [zone, setZone] = useState("");
  const [checking, setChecking] = useState(false);

  useEffect(() => {
    let cancelled = false;
    api
      .domains()
      .then(async (ds) => {
        const primary = ds.find((d) => d.isPrimary) ?? ds[0];
        if (!primary || cancelled) return;
        setDomainId(primary.id);
        const dns = await api.dns(primary.id);
        if (cancelled) return;
        setRecords(dns.records);
        setZone(dns.zoneFile);
      })
      .catch(() => {
        if (!cancelled) setRecords([]);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function checkNow() {
    if (!domainId) return;
    setChecking(true);
    try {
      const dns = await api.checkDns(domainId);
      setRecords(dns.records);
      const left = dns.records.filter((r) => r.required && r.status === "fail").length;
      if (left === 0) toast.success("All required records found. Moving on.");
      else toast.info(`${left} required record${left === 1 ? " is" : "s are"} still missing. DNS can take a few minutes to spread.`);
      await onRecheck();
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setChecking(false);
    }
  }

  const todo = (records ?? []).filter((r) => r.required && r.status === "fail");

  return (
    <div className="mt-3">
      {records === null ? (
        <p className="text-xs text-muted-foreground">Loading the records…</p>
      ) : todo.length > 0 ? (
        <ul className="divide-y divide-border/50 overflow-hidden rounded-md border border-border/60 bg-card">
          {todo.map((r) => (
            <li key={r.id} className="flex items-start gap-3 px-3 py-2.5">
              <span className="mt-0.5 w-12 shrink-0 rounded border border-border/70 px-1 py-px text-center font-mono text-[10px] text-foreground/80">{r.type}</span>
              <div className="min-w-0 flex-1">
                <p className="break-all font-mono text-xs text-foreground/90">{r.name}</p>
                <p className="mt-0.5 line-clamp-2 break-all font-mono text-[11px] text-muted-foreground" title={r.expected}>
                  {r.expected}
                </p>
              </div>
              <CopyButton value={r.expected} label={`${r.type} value for ${r.name}`} />
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-xs text-muted-foreground">Every required record already resolves — the next check will pick it up.</p>
      )}
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Button size="sm" onClick={() => void checkNow()} disabled={checking || !domainId}>
          <RefreshCw className={cn("h-3.5 w-3.5", checking && "animate-spin motion-reduce:animate-none")} />
          {checking ? "Checking…" : "Check now"}
        </Button>
        {zone && <CopyButton value={zone} label="zone file" showText text="Copy zone file" />}
        {domainId && (
          <Link
            to={`/console/domains/${domainId}`}
            className="ml-auto inline-flex items-center gap-1 rounded-sm text-xs text-muted-foreground underline-offset-2 transition-colors hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            Full DNS table <ArrowRight aria-hidden className="h-3 w-3" />
          </Link>
        )}
      </div>
    </div>
  );
}
