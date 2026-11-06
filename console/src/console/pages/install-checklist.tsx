import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import {
  ArrowRight,
  Check,
  ChevronRight,
  Hourglass,
  Loader2,
  Minus,
  RefreshCw,
  X,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Input } from "@/design/vendor/ui/input";
import { Label } from "@/design/vendor/ui/label";
import { Card } from "@/design/vendor/admin/ui";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { FirstMailboxResponse, DnsRecord, InstallState, InstallStep, StepStatus } from "@/api/types";
import { CopyButton } from "@/components/copy-button";
import { FieldError } from "@/components/kit";
import { StatusBadge, toneText, type Tone } from "@/components/status";
import { fmtDateTime } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useSession } from "../session";
import { CertificateProblem, useCertificate } from "../certificate";

const STATUS: Record<StepStatus, { label: string; tone: Tone }> = {
  done: { label: "Done", tone: "ok" },
  running: { label: "Working on it", tone: "muted" },
  waiting: { label: "Waiting for you", tone: "warn" },
  failed: { label: "Stopped", tone: "bad" },
  pending: { label: "Up next", tone: "muted" },
  skipped: { label: "Skipped", tone: "muted" },
};

/** The installer's steps, grouped the way a person thinks about them. */
const PHASES: { title: string; ids: string[] }[] = [
  { title: "This server", ids: ["preflight", "inputs", "host_record", "firewall"] },
  { title: "Mail engine", ids: ["install_engine", "bootstrap_engine", "edge_tls"] },
  { title: "On the internet", ids: ["certificate", "dns", "ptr"] },
  { title: "Your mail", ids: ["first_mailbox", "done"] },
];

const isActive = (s: InstallState) => s.steps.some((x) => x.status === "running" || x.status === "waiting");
const isFinished = (s: InstallState) => s.steps.length > 0 && s.steps.every((x) => x.status === "done" || x.status === "skipped");
const isDone = (x: InstallStep) => x.status === "done" || x.status === "skipped";

/**
 * Setup, live: progress, the steps grouped into four phases, and one card for
 * the step that needs attention now, with its action right there (Try now,
 * Check now, the records to copy, the first mailbox). Polls GET /install every
 * 3 s while a step is running or waiting.
 */
export function InstallChecklist({
  initial,
  onComplete,
  onMailboxCreated,
}: {
  initial: InstallState;
  onComplete: () => void;
  onMailboxCreated: (r: FirstMailboxResponse) => void;
}) {
  const [state, setState] = useState<InstallState>(initial);
  const completed = useRef(false);

  const prev = useRef<InstallState>(initial);
  const poll = useCallback(async () => {
    try {
      const next = await api.install();
      if (next.steps.length) {
        const was = prev.current.steps.find((x) => x.id === "certificate")?.status;
        const now = next.steps.find((x) => x.id === "certificate")?.status;
        if (was && was !== "done" && now === "done") toast.success("Certificate issued: browsers and mail apps now trust this server.");
        prev.current = next;
        setState(next);
      }
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

  const steps = state.steps;
  const doneCount = steps.filter(isDone).length;
  const currentIndex = steps.findIndex((s) => s.status === "waiting" || s.status === "running" || s.status === "failed");
  const current = currentIndex >= 0 ? steps[currentIndex] : undefined;
  const upNext = steps.filter((s, i) => i > currentIndex && s.status === "pending").slice(0, 3);

  return (
    <Card className="mb-6 overflow-hidden p-0">
      {/* Progress */}
      <div className="border-b border-border/50 px-5 pb-5 pt-5 sm:px-6">
        <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
          <p className="text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
            Setup · {doneCount} of {steps.length} done
          </p>
          {active && (
            <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground" aria-live="off">
              <span aria-hidden className="relative flex h-1.5 w-1.5">
                <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-foreground/30 motion-reduce:animate-none" />
                <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-foreground/50" />
              </span>
              Checking every few seconds
            </p>
          )}
        </div>
        <ol className="mt-3 flex gap-1" aria-label={`${doneCount} of ${steps.length} steps done`}>
          {steps.map((s) => (
            <li
              key={s.id}
              title={s.title}
              className={cn(
                "h-1.5 flex-1 rounded-full",
                isDone(s)
                  ? "bg-foreground/70"
                  : s.status === "waiting" || s.status === "running"
                    ? "bg-torii"
                    : s.status === "failed"
                      ? "bg-torii/60"
                      : "bg-foreground/10",
              )}
            />
          ))}
        </ol>
      </div>

      <div className="grid lg:grid-cols-[17rem_minmax(0,1fr)]">
        {/* Steps, by phase (folded away on small screens) */}
        <nav aria-label="Install steps" className="border-b border-border/50 lg:border-b-0 lg:border-r">
          <details className="group lg:hidden">
            <summary className="flex cursor-pointer list-none items-center gap-1.5 px-5 py-3 text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring sm:px-6 [&::-webkit-details-marker]:hidden">
              <ChevronRight aria-hidden className="h-3.5 w-3.5 transition-transform group-open:rotate-90 motion-reduce:transition-none" />
              All steps
            </summary>
            <div className="px-5 pb-4 sm:px-6">
              <PhaseList steps={steps} />
            </div>
          </details>
          <div className="hidden px-5 py-5 lg:block">
            <PhaseList steps={steps} />
          </div>
        </nav>

        {/* The step that needs attention now */}
        <div className="min-w-0 px-5 py-5 sm:px-6 sm:py-6">
          {current ? (
            <CurrentStep
              step={current}
              number={currentIndex + 1}
              total={steps.length}
              upNext={upNext}
              onRecheck={poll}
              onMailboxCreated={onMailboxCreated}
            />
          ) : (
            <p className="text-sm text-muted-foreground">{isFinished(state) ? "Every step is done." : "Starting the next step…"}</p>
          )}
          <InstallLog state={state} />
        </div>
      </div>
    </Card>
  );
}

function PhaseList({ steps }: { steps: InstallStep[] }) {
  const byId = new Map(steps.map((s, i) => [s.id, { step: s, n: i + 1 }]));
  const known = new Set(PHASES.flatMap((p) => p.ids));
  const phases = [...PHASES, { title: "Other", ids: steps.filter((s) => !known.has(s.id)).map((s) => s.id) }].filter((p) =>
    p.ids.some((id) => byId.has(id)),
  );
  return (
    <div className="space-y-4">
      {phases.map((phase) => {
        const items = phase.ids.map((id) => byId.get(id)).filter((x): x is { step: InstallStep; n: number } => !!x);
        const allDone = items.every((x) => isDone(x.step));
        return (
          <div key={phase.title}>
            <p className="mb-1.5 flex items-center gap-1.5 text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
              {phase.title}
              {allDone && <Check aria-label="done" className={cn("h-3 w-3", toneText("ok"))} strokeWidth={2.25} />}
            </p>
            <ol className="space-y-0.5">
              {items.map(({ step, n }) => (
                <PhaseItem key={step.id} step={step} n={n} />
              ))}
            </ol>
          </div>
        );
      })}
    </div>
  );
}

const ITEM_ICON: Record<StepStatus, LucideIcon | null> = {
  done: Check,
  skipped: Minus,
  running: Loader2,
  waiting: Hourglass,
  failed: X,
  pending: null,
};

function PhaseItem({ step, n }: { step: InstallStep; n: number }) {
  const Icon = ITEM_ICON[step.status];
  const now = step.status === "waiting" || step.status === "running" || step.status === "failed";
  return (
    <li
      className={cn(
        "relative flex items-start gap-2.5 rounded-md py-1.5 pl-2.5 pr-2 text-[13px] leading-snug",
        now ? "bg-foreground/[0.05] text-foreground" : isDone(step) ? "text-foreground/70" : "text-muted-foreground",
      )}
      aria-current={now ? "step" : undefined}
      title={step.detail ?? undefined}
    >
      {now && <span aria-hidden className="absolute inset-y-1.5 left-0 w-0.5 rounded-full bg-torii" />}
      <span
        aria-hidden
        className={cn(
          "mt-px grid h-4 w-4 shrink-0 place-items-center rounded-full text-[9px] tabular-nums",
          step.status === "done" && "bg-foreground/80 text-background",
          step.status === "skipped" && "bg-foreground/15 text-foreground/70",
          step.status === "pending" && "border border-foreground/20",
          now && "text-torii",
        )}
      >
        {Icon ? <Icon className={cn("h-3 w-3", step.status === "running" && "animate-spin motion-reduce:animate-none")} strokeWidth={2.5} /> : n}
      </span>
      <span className={cn("min-w-0", now && "font-medium")}>{step.title}</span>
      <span className="sr-only">: {STATUS[step.status].label}</span>
    </li>
  );
}

function CurrentStep({
  step,
  number,
  total,
  upNext,
  onRecheck,
  onMailboxCreated,
}: {
  step: InstallStep;
  number: number;
  total: number;
  upNext: InstallStep[];
  onRecheck: () => Promise<void>;
  onMailboxCreated: (r: FirstMailboxResponse) => void;
}) {
  const meta = STATUS[step.status];
  const waiting = step.status === "waiting";
  return (
    <section aria-labelledby="current-step-title">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <p className="text-[11px] tabular-nums text-muted-foreground">
          Step {number} of {total}
        </p>
        <StatusBadge tone={meta.tone} icon={step.status === "running" ? Loader2 : step.status === "waiting" ? Hourglass : undefined}>
          {meta.label}
        </StatusBadge>
      </div>
      <h2 id="current-step-title" className="mt-2 font-serif text-[24px] leading-tight tracking-tight sm:text-[26px]">
        {step.title}
      </h2>

      <div className="mt-4 max-w-3xl">
        {step.id === "certificate" ? (
          <CertificateTodo fallback={step.detail} onRecheck={onRecheck} />
        ) : step.id === "first_mailbox" && waiting ? (
          <FirstMailboxTodo onCreated={onMailboxCreated} onRecheck={onRecheck} />
        ) : step.id === "dns" && waiting ? (
          <>
            {step.detail && <p className="text-sm leading-relaxed text-foreground/90">{step.detail}</p>}
            <DnsTodo onRecheck={onRecheck} />
          </>
        ) : step.id === "ptr" && waiting ? (
          <PtrTodo detail={step.detail} />
        ) : (
          step.detail && <p className="text-sm leading-relaxed text-foreground/90 [overflow-wrap:anywhere]">{step.detail}</p>
        )}
        {step.status === "failed" && (
          <p className="mt-3 text-xs text-muted-foreground">
            Fix the cause, then run <code className="font-mono text-foreground/80">sudo yunori-mail install --resume</code> on the server.
          </p>
        )}
      </div>

      {upNext.length > 0 && (
        <p className="mt-6 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-xs text-muted-foreground">
          <span>Then</span>
          {upNext.map((s, i) => (
            <span key={s.id} className="inline-flex items-center gap-1.5">
              {i > 0 && <ArrowRight aria-hidden className="h-3 w-3 opacity-60" />}
              <span className="text-foreground/75">{s.title}</span>
            </span>
          ))}
        </p>
      )}
    </section>
  );
}

/** Reverse DNS lives at the VPS provider, not the DNS host: say exactly what to set. */
function PtrTodo({ detail }: { detail: string | null }) {
  const { session } = useSession();
  const host = session.server.hostname;
  const ip = detail?.match(/reverse DNS for (\S+) to/i)?.[1] ?? null;
  return (
    <div className="space-y-3">
      <p className="text-sm leading-relaxed text-foreground/90">
        Receiving servers check that this server's address names it back. Set it in your <strong className="font-medium">VPS provider's</strong>{" "}
        control panel (often “Networking” or “Reverse DNS”), not at your DNS host.
      </p>
      <div className="grid gap-2 sm:grid-cols-2">
        {ip && <CopyField label="IP address" value={ip} />}
        <CopyField label="Reverse DNS (PTR) name" value={host} />
      </div>
      {detail && <p className="text-xs leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">{detail}</p>}
    </div>
  );
}

function CopyField({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-border/60 bg-card px-3 py-2">
      <p className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground">{label}</p>
      <div className="mt-0.5 flex items-center justify-between gap-2">
        <code className="min-w-0 break-all font-mono text-[13px] text-foreground">{value}</code>
        <CopyButton value={value} label={label} />
      </div>
    </div>
  );
}

function InstallLog({ state }: { state: InstallState }) {
  return (
    <details className="group mt-6 border-t border-border/50 pt-3">
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
  );
}

/** Automatic HTTPS has not succeeded yet: say why, and offer "Try now". */
function CertificateTodo({ fallback, onRecheck }: { fallback: string | null; onRecheck: () => Promise<void> }) {
  // When an attempt succeeds, move the checklist on without waiting for its next tick.
  const { cert, busy, tryNow } = useCertificate({ live: true, onIssued: () => void onRecheck() });
  return <CertificateProblem cert={cert} busy={busy} onTryNow={() => void tryNow()} fallback={fallback} />;
}

/** The last thing setup needs: an address. Creates it with postmaster@ and abuse@. */
function FirstMailboxTodo({ onCreated, onRecheck }: { onCreated: (r: FirstMailboxResponse) => void; onRecheck: () => Promise<void> }) {
  const { session } = useSession();
  const domain = session.server.primaryDomain ?? "";
  const [local, setLocal] = useState("hello");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    const v = local.trim().toLowerCase();
    if (!/^[a-z0-9](?:[a-z0-9._+-]{0,62}[a-z0-9])?$/.test(v)) {
      setError("Use letters, digits, dots, dashes or underscores.");
      return;
    }
    setBusy(true);
    try {
      const r = await api.createFirstMailbox({ local: v, displayName: name.trim() || undefined });
      onCreated(r);
      toast.success(`${r.mailbox.address} is ready.`);
      await onRecheck();
    } catch (err) {
      setError(err instanceof ApiRequestError && err.field === "local" ? err.message : errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={onSubmit} noValidate className="space-y-3">
      <p className="text-sm leading-relaxed text-foreground">
        Pick the address for your first mailbox. <span className="font-mono text-[13px]">postmaster@</span> and{" "}
        <span className="font-mono text-[13px]">abuse@</span>, which every mail domain needs, deliver to it too.
      </p>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="first-local">Address</Label>
          <div className="flex items-center rounded-md border border-input bg-card focus-within:ring-1 focus-within:ring-ring">
            <Input
              id="first-local"
              value={local}
              onChange={(e) => setLocal(e.target.value)}
              autoComplete="off"
              spellCheck={false}
              className="border-0 bg-transparent font-mono shadow-none focus-visible:ring-0"
              aria-invalid={!!error}
              aria-describedby={error ? "first-local-err" : undefined}
            />
            <span className="shrink-0 pr-3 font-mono text-[13px] text-muted-foreground">@{domain}</span>
          </div>
          <FieldError id="first-local-err">{error}</FieldError>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="first-name">Display name (optional)</Label>
          <Input id="first-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Front desk" />
        </div>
      </div>
      <Button type="submit" size="sm" variant="torii" disabled={busy}>
        {busy ? "Creating…" : "Create mailbox"}
      </Button>
    </form>
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
    <div className="mt-4">
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
