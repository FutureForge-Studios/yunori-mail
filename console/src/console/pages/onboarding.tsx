import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Link, Navigate, useNavigate } from "react-router-dom";
import {
  ArrowLeft,
  ArrowRight,
  ChevronDown,
  CloudCog,
  ExternalLink,
  Globe,
  Hourglass,
  Loader2,
  Minus,
  RefreshCw,
  ScrollText,
  Server,
  ShieldCheck,
  Terminal,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type {
  DnsRecord,
  Domain,
  FirstMailboxResponse,
  InstallStep,
  OnboardingState,
  RelayProvider,
  SendingResponse,
  StepStatus,
} from "@/api/types";
import { CopyButton } from "@/components/copy-button";
import { PasswordInput } from "@/components/password-input";
import { CheckRow } from "@/components/status";
import { ThemeToggle } from "@/components/theme-toggle";
import { YunoriMark } from "@/components/yunori-logo";
import { toast } from "@/lib/toast";
import { Badge, Chip, StatusBadge } from "@/ui/badge";
import { Button } from "@/ui/button";
import { Card, CardBody, CardFooter, CardHeader } from "@/ui/card";
import { Callout, EmptyState, FormError, LoadError, Skeleton } from "@/ui/feedback";
import { Field, Input, Label, RadioCard, Select } from "@/ui/form";
import { PageContainer, usePageTitle } from "@/ui/page";
import { Step, Steps, type StepState } from "@/ui/steps";
import { TONE_TEXT, type Tone } from "@/ui/tone";
import { useSession } from "../session";
import { CertificateProblem, useCertificate } from "../certificate";
import { DnsProviderCard } from "./domain-dns-provider";
import { SetupComplete } from "./setup-complete";

/**
 * The browser onboarding: everything the terminal used to ask and wait for,
 * one step at a time. The server owns the state (GET /onboarding) and decides
 * every gate against public DNS and the certificate authority; this page only
 * answers, explains, and asks for a re-check (it re-checks by itself every
 * minute too). Reloading, or coming back days later, resumes where it was.
 */

type WizardId = "inputs" | "sending" | "host_record" | "certificate" | "dns" | "ptr" | "first_mailbox" | "done";

const WIZARD: { id: WizardId; title: string; lead: string }[] = [
  { id: "inputs", title: "Your mail domain", lead: "The domain your addresses end in, the mail server's own name, and where alerts go." },
  { id: "sending", title: "How mail goes out", lead: "Deliver directly from this server, or hand outgoing mail to a relay provider." },
  { id: "host_record", title: "Point the mail hostname here", lead: "One DNS record so the world can find this server by name." },
  { id: "certificate", title: "A trusted certificate", lead: "So browsers and mail apps trust the mail hostname. Automatic, with Let's Encrypt." },
  { id: "dns", title: "DNS records", lead: "MX, SPF, DKIM and DMARC: how other mail servers find and trust your domain." },
  { id: "ptr", title: "Reverse DNS", lead: "This server's address must name the mail hostname back. Big providers check it." },
  { id: "first_mailbox", title: "Your first mailbox", lead: "An address to read mail at. postmaster@ and abuse@ deliver to it too." },
  { id: "done", title: "Done", lead: "Your mail server is ready." },
];

const isDone = (s: StepStatus | undefined) => s === "done" || s === "skipped";
const GATES: WizardId[] = ["host_record", "certificate", "dns", "ptr"];

function stepOf(state: OnboardingState, id: string): InstallStep | undefined {
  return state.steps.find((s) => s.id === id);
}

/** The first onboarding step the server has not finished. */
function currentStep(state: OnboardingState): WizardId {
  if (state.complete) return "done";
  return WIZARD.find((w) => w.id !== "done" && !isDone(stepOf(state, w.id)?.status))?.id ?? "done";
}

export default function OnboardingPage() {
  usePageTitle("Set up");
  const { session, refresh } = useSession();
  const navigate = useNavigate();
  const [state, setState] = useState<OnboardingState | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [viewing, setViewing] = useState<WizardId | null>(null);
  const [created, setCreated] = useState<FirstMailboxResponse | null>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);

  const load = useCallback(async () => {
    try {
      setState(await api.onboarding());
      setError(null);
    } catch (err) {
      setError(err);
    }
  }, []);

  /** An answer was accepted: show whatever the server says is next. */
  const accept = useCallback((next: OnboardingState) => {
    setState(next);
    setViewing(null);
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const current = state ? currentStep(state) : null;
  const gate = !!current && GATES.includes(current);

  // The session carries the server's names (mail apps, the portal link): keep it current.
  const mailHost = state?.mailHostname;
  const complete = state?.complete;
  useEffect(() => {
    if (mailHost) refresh().catch(() => {});
  }, [mailHost, complete, refresh]);

  // A cheap read every few seconds while a check is pending; the server's own
  // re-check runs every minute (and on "Check now").
  useEffect(() => {
    if (!state || state.complete) return;
    const t = window.setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, gate ? 4000 : 15000);
    return () => window.clearInterval(t);
  }, [state, gate, load]);
  useEffect(() => {
    if (!gate) return;
    const t = window.setInterval(() => {
      if (document.visibilityState !== "visible") return;
      api
        .checkOnboarding()
        .then(setState)
        .catch(() => {});
    }, 60_000);
    return () => window.clearInterval(t);
  }, [gate]);

  // Move focus to the new step's heading when the step changes.
  const shown: WizardId | null = viewing ?? current;
  const firstShown = useRef(true);
  useEffect(() => {
    if (firstShown.current) {
      firstShown.current = false;
      return;
    }
    headingRef.current?.focus();
  }, [shown]);

  if (session.setupRequired) return <Navigate to="/console/setup" replace />;

  if (error && !state) {
    return (
      <Frame>
        <LoadError error={error} onRetry={() => void load()} what="Setup" />
      </Frame>
    );
  }
  if (!state || !shown || !current) {
    return (
      <Frame>
        <div aria-busy="true" className="grid gap-6 lg:grid-cols-[17rem_minmax(0,1fr)] lg:gap-12">
          <Skeleton className="hidden h-[26rem] rounded-xl lg:block" />
          <div>
            <Skeleton className="h-3 w-24" />
            <Skeleton className="mt-3 h-9 w-full max-w-md" />
            <Skeleton className="mt-3 h-4 w-full max-w-xl" />
            <Skeleton className="mt-8 h-72 rounded-xl" />
          </div>
        </div>
      </Frame>
    );
  }

  if (!state.terminalDone) {
    return (
      <Frame>
        <EmptyState
          icon={Terminal}
          title="The terminal part has not finished"
          hint={
            <>
              Run <code className="font-mono text-[12px] text-foreground/85">sudo yunori-mail install</code> on the server again: it finishes installing the mail
              engine, then this page continues.
            </>
          }
        />
      </Frame>
    );
  }

  const meta = WIZARD.find((w) => w.id === shown)!;
  const index = WIZARD.findIndex((w) => w.id === shown);
  const step = stepOf(state, shown);

  return (
    <Frame panel={state.panel.host}>
      <div className="grid gap-6 lg:grid-cols-[17rem_minmax(0,1fr)] lg:gap-12">
        <Stepper state={state} current={current} shown={shown} onOpen={(id) => setViewing(id === current ? null : id)} />

        <section aria-labelledby="onboarding-step-title" className="min-w-0 max-w-3xl">
          {viewing && viewing !== current && (
            <Button variant="ghost" size="sm" onClick={() => setViewing(null)} className="-ml-2 mb-3 h-7 px-2 text-xs text-muted-foreground">
              <ArrowLeft aria-hidden className="h-3.5 w-3.5" /> Back to “{WIZARD.find((w) => w.id === current)?.title}”
            </Button>
          )}
          <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5">
            <p className="text-xs font-medium tabular-nums text-muted-foreground">
              {/* Seven real steps; "Done" is the finish, not an eighth step. */}
              {shown === "done" ? "Finish" : `Step ${index + 1} of ${WIZARD.length - 1}`}
            </p>
            {step && shown !== "done" && <StepBadge status={step.status} current={shown === current} />}
          </div>
          <h1
            id="onboarding-step-title"
            ref={headingRef}
            tabIndex={-1}
            className="mt-2 font-serif text-[28px] leading-[1.12] tracking-[-0.015em] text-foreground [overflow-wrap:anywhere] focus:outline-none sm:text-[34px]"
          >
            {meta.title}
          </h1>
          <p className="mt-2 max-w-2xl text-sm leading-relaxed text-muted-foreground">{meta.lead}</p>

          <div key={shown} className="mt-7 animate-rise-in motion-reduce:animate-none">
            {shown === "inputs" && <DomainStep state={state} onSaved={accept} />}
            {shown === "sending" && <SendingStep state={state} onSaved={accept} />}
            {shown === "host_record" && <HostStep state={state} onState={setState} />}
            {shown === "certificate" && <CertificateStep state={state} onRecheck={load} />}
            {shown === "dns" && <DnsStep state={state} onState={setState} />}
            {shown === "ptr" && <PtrStep state={state} onState={setState} onRelay={() => setViewing("sending")} />}
            {shown === "first_mailbox" && (
              <MailboxStep
                state={state}
                onCreated={async (r) => {
                  setCreated(r);
                  await load();
                }}
              />
            )}
            {shown === "done" && (
              <DoneStep
                state={state}
                created={created}
                onFinish={async () => {
                  await refresh();
                  navigate("/console", { replace: true });
                }}
              />
            )}
          </div>
          {shown !== "done" && <SetupLog state={state} />}
        </section>
      </div>
    </Frame>
  );
}

// ── frame & stepper ──────────────────────────────────────────────────────────

function Frame({ children, panel }: { children: ReactNode; panel?: string }) {
  return (
    <div className="min-h-dvh bg-background">
      <header className="sticky top-0 z-30 border-b border-border bg-background/85 backdrop-blur-md supports-[backdrop-filter]:bg-background/70">
        <div className="mx-auto flex h-[var(--topbar-height)] max-w-6xl items-center gap-3 px-4 sm:px-6 lg:px-8">
          <div className="flex min-w-0 items-center gap-2.5">
            <YunoriMark className="h-6 w-6 shrink-0" />
            <span className="whitespace-nowrap text-sm font-semibold tracking-tight">Yunori Mail</span>
            <span aria-hidden className="h-4 w-px shrink-0 bg-border-strong/70" />
            <span className="whitespace-nowrap text-sm text-muted-foreground">Setup</span>
            {panel && <Chip className="ml-1 hidden md:inline-flex">{panel}</Chip>}
          </div>
          <div className="ml-auto flex shrink-0 items-center gap-1">
            <Button asChild variant="ghost" size="sm" className="h-8 px-2.5 text-[13px] text-muted-foreground">
              <Link to="/console">Console</Link>
            </Button>
            <ThemeToggle />
          </div>
        </div>
      </header>
      <main>
        <PageContainer width="default" className="sm:pt-10">
          {children}
        </PageContainer>
      </main>
    </div>
  );
}

function StepBadge({ status, current }: { status: StepStatus; current: boolean }) {
  const m: Record<StepStatus, { label: string; tone: Tone; icon?: LucideIcon }> = {
    done: { label: "Done", tone: "ok" },
    skipped: { label: "Skipped for now", tone: "muted", icon: Minus },
    running: { label: "Checking", tone: "info", icon: Loader2 },
    waiting: { label: current ? "Your turn" : "Waiting", tone: "warn", icon: Hourglass },
    failed: { label: "Needs attention", tone: "bad" },
    pending: { label: current ? "Your turn" : "Up next", tone: current ? "warn" : "muted" },
  };
  const { label, tone, icon } = m[status];
  return (
    <StatusBadge tone={tone} icon={icon}>
      {label}
    </StatusBadge>
  );
}

/** How a wizard step shows in the step list (the ui kit's Steps vocabulary). */
function railState(id: WizardId, st: StepStatus, current: WizardId): StepState {
  if (st === "done") return "done";
  if (st === "skipped") return "skipped";
  if (st === "failed") return "failed";
  if (id !== current) return "pending";
  // A gate waits on the outside world (DNS, the CA); every other step waits on you.
  return GATES.includes(id) && (st === "waiting" || st === "running") ? "waiting" : "current";
}

const RAIL_WORD: Partial<Record<StepState, string>> = {
  skipped: "Skipped",
  failed: "Needs attention",
  waiting: "Your turn",
  current: "Your turn",
};

function Stepper({
  state,
  current,
  shown,
  onOpen,
}: {
  state: OnboardingState;
  current: WizardId;
  shown: WizardId;
  onOpen: (id: WizardId) => void;
}) {
  const doneCount = WIZARD.filter((w) => w.id !== "done" && isDone(stepOf(state, w.id)?.status)).length;
  const total = WIZARD.length - 1;
  const shownIndex = WIZARD.findIndex((w) => w.id === shown);

  const list = (
    <Steps label="Setup steps">
      {WIZARD.map((w, i) => {
        const st: StepStatus = w.id === "done" ? (state.complete ? "done" : "pending") : (stepOf(state, w.id)?.status ?? "pending");
        const reachable = isDone(st) || w.id === current;
        const active = w.id === shown;
        const rs = railState(w.id, st, current);
        return (
          <Step
            key={w.id}
            index={w.id === "done" ? undefined : i + 1}
            last={i === WIZARD.length - 1}
            state={rs}
            meta={w.id === "done" ? undefined : st === "running" && w.id === current ? "Checking" : RAIL_WORD[rs]}
            title={
              <button
                type="button"
                disabled={!reachable}
                onClick={() => onOpen(w.id)}
                aria-current={active ? "location" : undefined}
                className={cn(
                  "-mx-1 rounded-md px-1 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 disabled:cursor-default",
                  active
                    ? "text-foreground underline decoration-torii decoration-2 underline-offset-[5px]"
                    : reachable && "hover:text-foreground hover:underline hover:decoration-border-strong hover:underline-offset-[5px]",
                )}
              >
                {w.title}
                <span className="sr-only">: {isDone(st) ? (st === "skipped" ? "skipped" : "done") : w.id === current ? "current step" : "not yet"}</span>
              </button>
            }
          />
        );
      })}
    </Steps>
  );

  const progress = (
    <div className="mt-3 flex gap-1" aria-hidden>
      {WIZARD.filter((w) => w.id !== "done").map((w) => (
        <span
          key={w.id}
          className={cn(
            "h-1.5 flex-1 rounded-full transition-colors motion-reduce:transition-none",
            isDone(stepOf(state, w.id)?.status) ? "bg-foreground/70" : w.id === current ? "bg-torii" : "bg-foreground/10",
          )}
        />
      ))}
    </div>
  );

  return (
    <nav aria-label="Setup progress" className="min-w-0">
      {/* Phones and tablets: a compact progress card; the full list on demand. */}
      <Card padded={false} className="lg:hidden">
        <details className="group">
          <summary className="flex cursor-pointer list-none flex-col rounded-xl px-4 py-3.5 transition-colors hover:bg-foreground/[0.02] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 [&::-webkit-details-marker]:hidden">
            <span className="flex items-center justify-between gap-3">
              <span className="min-w-0 text-[13px]">
                <span className="font-medium text-foreground">
                  {doneCount} of {total} done
                </span>
                <span className="text-muted-foreground">{shown === "done" ? " · finish" : ` · step ${shownIndex + 1} of ${total}`}</span>
              </span>
              <span className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
                <span className="group-open:hidden">All steps</span>
                <span className="hidden group-open:inline">Hide</span>
                <ChevronDown aria-hidden className="h-3.5 w-3.5 transition-transform group-open:rotate-180 motion-reduce:transition-none" />
              </span>
            </span>
            {progress}
          </summary>
          <div className="border-t border-border/70 px-4 py-4">{list}</div>
        </details>
      </Card>

      {/* Desktop: the whole process at a glance, beside the step. */}
      <div className="sticky top-[calc(var(--topbar-height)+2.5rem)] hidden lg:block">
        <Card padded={false}>
          <div className="border-b border-border/70 px-5 pb-4 pt-5">
            <div className="flex items-baseline justify-between gap-3">
              <p className="text-sm font-semibold text-foreground">Setup</p>
              <p className="text-xs tabular-nums text-muted-foreground">
                {doneCount} of {total} done
              </p>
            </div>
            {progress}
          </div>
          <div className="px-5 py-5">{list}</div>
          <CardFooter className="text-xs leading-relaxed text-muted-foreground">
            You can close this page at any time. The server keeps checking by itself and picks up where you left off.
          </CardFooter>
        </Card>
      </div>
    </nav>
  );
}

function SetupLog({ state }: { state: OnboardingState }) {
  const lines = state.logs.slice(-40);
  if (!lines.length) return null;
  return (
    <details className="group mt-10 rounded-xl border border-border bg-card/50">
      <summary className="flex cursor-pointer list-none items-center gap-2.5 rounded-xl px-4 py-3 text-[13px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 [&::-webkit-details-marker]:hidden">
        <ScrollText aria-hidden className="h-4 w-4" strokeWidth={1.75} />
        <span className="flex-1">Setup log</span>
        <span className="text-xs tabular-nums">
          {lines.length} entr{lines.length === 1 ? "y" : "ies"}
        </span>
        <ChevronDown aria-hidden className="h-3.5 w-3.5 transition-transform group-open:rotate-180 motion-reduce:transition-none" />
      </summary>
      <ul
        className="max-h-72 space-y-1.5 overflow-auto border-t border-border/70 px-4 py-3 font-mono text-[11.5px] leading-relaxed"
        tabIndex={0}
        aria-label="Setup log"
      >
        {lines.map((l, i) => (
          <li key={i} className="flex gap-3">
            <time dateTime={l.at} className="shrink-0 tabular-nums text-muted-foreground">
              {new Date(l.at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
            </time>
            <span className={cn("min-w-0 [overflow-wrap:anywhere]", l.level === "info" ? "text-foreground/85" : TONE_TEXT[l.level === "warn" ? "warn" : "bad"])}>
              {l.message}
            </span>
          </li>
        ))}
      </ul>
    </details>
  );
}

// ── shared bits ──────────────────────────────────────────────────────────────

function CopyField({ label, value, className }: { label: string; value: string; className?: string }) {
  return (
    <div className={cn("min-w-0 rounded-lg border border-border bg-muted/35 py-2 pl-3 pr-1.5", className)}>
      <p className="text-xs text-muted-foreground">{label}</p>
      <div className="mt-0.5 flex items-center justify-between gap-2">
        <code className="min-w-0 break-all font-mono text-[13px] text-foreground">{value}</code>
        <CopyButton value={value} label={label} />
      </div>
    </div>
  );
}

/** The server's own words for a step (what it found, what to do). */
function Detail({ step }: { step: InstallStep | undefined }) {
  if (!step?.detail) return null;
  const done = isDone(step.status);
  const tone: Tone = step.status === "done" ? "ok" : step.status === "skipped" ? "muted" : step.status === "failed" ? "bad" : step.status === "running" ? "info" : "warn";
  return (
    <Callout tone={tone} icon={step.status === "waiting" || step.status === "pending" ? Hourglass : undefined} role={done ? undefined : "status"} className="[overflow-wrap:anywhere]">
      {step.detail}
    </Callout>
  );
}

/** The row under a step: a note on the left, the step's action on the right. */
function StepActions({ note, children }: { note?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex flex-col-reverse gap-3 pt-1 sm:flex-row sm:items-center sm:justify-between">
      <p className="text-xs leading-relaxed text-muted-foreground">{note}</p>
      <div className="flex shrink-0 flex-col gap-2 sm:flex-row sm:items-center">{children}</div>
    </div>
  );
}

/** "Check now" plus a note that the server re-checks every minute anyway. */
function Recheck({ onCheck, label = "Check now" }: { onCheck: () => Promise<void>; label?: string }) {
  const [busy, setBusy] = useState(false);
  return (
    <StepActions note="Checked against public DNS, again every minute by itself.">
      <Button
        onClick={async () => {
          setBusy(true);
          try {
            await onCheck();
          } finally {
            setBusy(false);
          }
        }}
        disabled={busy}
        className="w-full sm:w-auto"
      >
        <RefreshCw aria-hidden className={cn("h-3.5 w-3.5", busy && "animate-spin motion-reduce:animate-none")} />
        {busy ? "Checking…" : label}
      </Button>
    </StepActions>
  );
}

function useCheck(onState: (s: OnboardingState) => void) {
  return useCallback(async () => {
    try {
      onState(await api.checkOnboarding());
    } catch (err) {
      toast.error(errorMessage(err));
    }
  }, [onState]);
}

function usePrimaryDomain(state: OnboardingState): Domain | null {
  const [domain, setDomain] = useState<Domain | null>(null);
  const id = state.primaryDomainId;
  const name = state.primaryDomain;
  useEffect(() => {
    if (!id) return;
    let live = true;
    api
      .domains()
      .then((ds) => live && setDomain(ds.find((d) => d.id === id) ?? null))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [id, name]);
  return domain;
}

/** A card-shaped row that opens to more (the Cloudflare panel, optional records). */
function Disclosure({ icon: Icon, title, description, children }: { icon: LucideIcon; title: string; description?: string; children: ReactNode }) {
  return (
    <details className="group">
      <summary className="flex cursor-pointer list-none items-center gap-3 rounded-xl border border-border bg-card px-4 py-3 shadow-card transition-colors hover:border-border-strong focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 focus-visible:ring-offset-2 focus-visible:ring-offset-background [&::-webkit-details-marker]:hidden">
        <span aria-hidden className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-border bg-muted/50 text-icon">
          <Icon className="h-4 w-4" strokeWidth={1.75} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium text-foreground">{title}</span>
          {description && <span className="mt-0.5 block text-xs leading-relaxed text-muted-foreground">{description}</span>}
        </span>
        <ChevronDown aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180 motion-reduce:transition-none" />
      </summary>
      <div className="mt-3">{children}</div>
    </details>
  );
}

const CLOUDFLARE_HINT = "Paste a scoped API token; Yunori shows every change before it publishes.";

// ── 1. mail domain ───────────────────────────────────────────────────────────

type IdentityErrors = Partial<Record<"domain" | "hostname" | "adminEmail" | "form", string>>;

function DomainStep({ state, onSaved }: { state: OnboardingState; onSaved: (s: OnboardingState) => void }) {
  const initialDomain = state.identity?.domain ?? state.suggested.domain ?? "";
  const [domain, setDomain] = useState(initialDomain);
  const [hostname, setHostname] = useState(state.identity?.hostname ?? state.suggested.hostname ?? (initialDomain ? `mail.${initialDomain}` : ""));
  const [hostEdited, setHostEdited] = useState(!!state.identity);
  const [email, setEmail] = useState(state.identity?.adminEmail ?? state.suggested.adminEmail ?? "");
  const [errors, setErrors] = useState<IdentityErrors>({});
  const [busy, setBusy] = useState(false);

  const d = domain.trim().toLowerCase().replace(/\.$/, "");
  const parent = /^mail\.[^.]+\.[^.]+/.test(d) ? d.slice(5) : null;
  const laterDone = ["host_record", "certificate", "dns"].some((id) => isDone(stepOf(state, id)?.status));

  function changeDomain(v: string) {
    setDomain(v);
    if (!hostEdited) {
      const n = v.trim().toLowerCase().replace(/\.$/, "");
      setHostname(n ? `mail.${n}` : "");
    }
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    const local: IdentityErrors = {};
    if (!/^[a-z0-9-]+(\.[a-z0-9-]+)+$/i.test(d)) local.domain = "Enter a domain like example.com: no https://, no @.";
    if (!/^[a-z0-9-]+(\.[a-z0-9-]+)+\.?$/i.test(hostname.trim())) local.hostname = "Enter a host name like mail.example.com.";
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim())) local.adminEmail = "Enter an email address.";
    setErrors(local);
    if (Object.keys(local).length) return;
    setBusy(true);
    try {
      const next = await api.saveIdentity({ domain: d, hostname: hostname.trim(), adminEmail: email.trim() });
      toast.success(`${next.identity?.domain ?? d} is set up on the mail engine.`);
      onSaved(next);
    } catch (err) {
      if (err instanceof ApiRequestError && err.field && ["domain", "hostname", "adminEmail"].includes(err.field)) {
        setErrors({ [err.field]: err.message });
      } else {
        setErrors({ form: errorMessage(err) });
      }
    } finally {
      setBusy(false);
    }
  }

  const panelNote =
    state.panel.kind === "domain"
      ? null
      : `Right now the console is at ${state.panel.host}. Once the mail hostname has its certificate, the console also opens at https://${hostname || "mail.<your domain>"}/console.`;

  return (
    <form onSubmit={onSubmit} noValidate className="space-y-5">
      <Card padded={false}>
        <CardBody className="space-y-5 pt-5">
          <div className="grid gap-x-4 gap-y-5 sm:grid-cols-2">
            <Field label="Mail domain" htmlFor="ob-domain" hint="The part after @ in every address." error={errors.domain}>
              <Input
                id="ob-domain"
                value={domain}
                onChange={(e) => changeDomain(e.target.value)}
                placeholder="example.com"
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                autoFocus={!state.identity}
                className="font-mono"
                aria-invalid={!!errors.domain}
                aria-describedby={errors.domain ? "ob-domain-err" : undefined}
              />
              {parent && !errors.domain && (
                <p className="text-xs leading-relaxed text-foreground/85">
                  Addresses would look like you@{d}.{" "}
                  <button
                    type="button"
                    className="rounded-sm font-medium underline underline-offset-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
                    onClick={() => changeDomain(parent)}
                  >
                    Use {parent} instead
                  </button>
                </p>
              )}
            </Field>
            <Field label="Mail server hostname" htmlFor="ob-host" hint="This server's own name: where mail is delivered and mail apps connect." error={errors.hostname}>
              <Input
                id="ob-host"
                value={hostname}
                onChange={(e) => {
                  setHostEdited(true);
                  setHostname(e.target.value);
                }}
                placeholder="mail.example.com"
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                className="font-mono"
                aria-invalid={!!errors.hostname}
                aria-describedby={errors.hostname ? "ob-host-err" : undefined}
              />
            </Field>
          </div>
          <Field
            label="Your email on another provider"
            htmlFor="ob-email"
            hint="For certificate notices and alerts. It must not depend on this server: if the server is down, you still hear about it."
            error={errors.adminEmail}
            errorId="ob-email-err"
          >
            <Input
              id="ob-email"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="you@gmail.com"
              autoComplete="email"
              className="sm:max-w-[calc(50%-0.5rem)]"
              aria-invalid={!!errors.adminEmail}
              aria-describedby={errors.adminEmail ? "ob-email-err" : undefined}
            />
          </Field>
        </CardBody>
        {d && /\./.test(d) && (
          <CardFooter className="block space-y-1 py-3.5 text-[13px] leading-relaxed text-muted-foreground">
            <p>
              Addresses look like <span className="font-mono text-foreground">you@{d}</span>. Other mail servers deliver to{" "}
              <span className="font-mono text-foreground">{hostname.trim() || `mail.${d}`}</span>, and mail apps connect there.
            </p>
            {panelNote && <p className="text-xs">{panelNote}</p>}
          </CardFooter>
        )}
      </Card>
      {(!d || !/\./.test(d)) && panelNote && <p className="text-xs leading-relaxed text-muted-foreground">{panelNote}</p>}
      {laterDone && <Callout tone="warn">Changing these checks the hostname, certificate and DNS steps again.</Callout>}
      <FormError>{errors.form}</FormError>
      <StepActions>
        <Button type="submit" variant="torii" loading={busy} className="w-full sm:w-auto">
          {busy ? "Saving…" : state.identity ? "Save" : "Continue"}
          {!busy && <ArrowRight aria-hidden className="h-3.5 w-3.5" />}
        </Button>
      </StepActions>
    </form>
  );
}

// ── 2. sending ───────────────────────────────────────────────────────────────

type RelayErrors = Partial<Record<"provider" | "host" | "port" | "username" | "password" | "region" | "form", string>>;

function SendingStep({ state, onSaved }: { state: OnboardingState; onSaved: (s: OnboardingState) => void }) {
  const [info, setInfo] = useState<SendingResponse | null>(null);
  const [mode, setMode] = useState<"direct" | "relay">(state.sending.confirmed ? state.sending.mode : state.sending.suggested);
  const [providerKey, setProviderKey] = useState("");
  const [region, setRegion] = useState("");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("587");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [errors, setErrors] = useState<RelayErrors>({});
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    api
      .sending()
      .then((s) => {
        if (!live) return;
        setInfo(s);
        const p = s.relay ? s.providers.find((x) => x.key === s.relay!.provider) : s.providers[0];
        if (p) {
          setProviderKey(p.key);
          setRegion(s.relay?.region ?? p.defaultRegion ?? "");
          setHost(s.relay?.host ?? p.host ?? "");
          setPort(String(s.relay?.port ?? p.port));
          setUsername(s.relay?.username ?? p.username ?? "");
        }
      })
      .catch((err) => live && setErrors({ form: errorMessage(err) }));
    return () => {
      live = false;
    };
  }, []);

  const provider: RelayProvider | undefined = info?.providers.find((p) => p.key === providerKey);
  const savedPassword = !!info?.relay?.hasPassword && info.relay.provider === providerKey;

  function pickProvider(key: string) {
    const p = info?.providers.find((x) => x.key === key);
    setProviderKey(key);
    if (!p) return;
    setRegion(p.defaultRegion ?? "");
    setHost(p.host ?? "");
    setPort(String(p.port));
    setUsername(p.username ?? "");
    setPassword("");
  }

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setErrors({});
    setBusy(true);
    try {
      if (mode === "relay") {
        if (!provider) throw new Error("Choose a provider.");
        await api.updateSending({
          mode: "relay",
          relay: {
            provider: provider.key,
            host: provider.hostTemplate ? "" : host.trim(),
            port: Number(port),
            username: username.trim(),
            password: password || undefined,
            region: provider.regions.length ? region : undefined,
          },
        });
      } else {
        await api.updateSending({ mode: "direct" });
      }
      setPassword("");
      toast.success(mode === "relay" ? `Mail goes out through ${provider?.label}.` : "This server delivers mail itself.");
      onSaved(await api.onboarding());
    } catch (err) {
      if (err instanceof ApiRequestError && err.field && ["provider", "host", "port", "username", "password", "region"].includes(err.field)) {
        setErrors({ [err.field]: err.message });
      } else {
        setErrors({ form: errorMessage(err, err instanceof Error ? err.message : undefined) });
      }
    } finally {
      setBusy(false);
    }
  }

  const blocked = state.sending.port25.status === "fail" || state.sending.port25.status === "warn";
  const suggested = <Badge tone="ok">Suggested</Badge>;
  return (
    <form onSubmit={onSubmit} noValidate className="space-y-5">
      <Card padded={false}>
        <ul className="px-5 py-1">
          <CheckRow check={state.sending.port25} compact />
        </ul>
      </Card>
      <fieldset>
        <legend className="sr-only">How this server sends mail</legend>
        <div className="grid gap-3 sm:grid-cols-2">
          <RadioCard
            name="ob-sending"
            value="direct"
            checked={mode === "direct"}
            onChange={() => setMode("direct")}
            title="Directly from this server"
            badge={state.sending.suggested === "direct" ? suggested : undefined}
            description="Needs outbound port 25 and reverse DNS. Nothing else to sign up for."
            className="h-full"
          />
          <RadioCard
            name="ob-sending"
            value="relay"
            checked={mode === "relay"}
            onChange={() => setMode("relay")}
            title="Through a relay provider"
            badge={state.sending.suggested === "relay" ? suggested : undefined}
            description={
              blocked
                ? "Your provider seems to block outbound port 25: a relay (Amazon SES, Resend, Postmark…) delivers for you. Receiving works either way."
                : "Amazon SES, Resend, Postmark, Mailgun… They deliver for you; reverse DNS becomes optional."
            }
            className="h-full"
          />
        </div>
      </fieldset>

      {mode === "relay" && (
        <Card padded={false} className="animate-rise-in motion-reduce:animate-none">
          <CardHeader
            icon={Server}
            title="Relay provider"
            description="The password is write-only: sealed on the server, never shown again."
          />
          <CardBody>
            {!info ? (
              <Skeleton className="h-32" />
            ) : (
              <div className="grid gap-x-4 gap-y-5 sm:grid-cols-2">
                <Field label="Provider" htmlFor="ob-provider" error={errors.provider}>
                  <Select id="ob-provider" value={providerKey} onChange={(e) => pickProvider(e.target.value)}>
                    {info.providers.map((p) => (
                      <option key={p.key} value={p.key}>
                        {p.label}
                      </option>
                    ))}
                  </Select>
                </Field>
                {provider && provider.regions.length > 0 && (
                  <Field label="Region" htmlFor="ob-region" error={errors.region}>
                    <Select id="ob-region" value={region} onChange={(e) => setRegion(e.target.value)}>
                      {provider.regions.map((r) => (
                        <option key={r} value={r}>
                          {r}
                        </option>
                      ))}
                    </Select>
                  </Field>
                )}
                {provider && !provider.hostTemplate && (
                  <Field label="SMTP host" htmlFor="ob-relay-host" error={errors.host}>
                    <Input id="ob-relay-host" value={host} onChange={(e) => setHost(e.target.value)} className="font-mono" spellCheck={false} aria-invalid={!!errors.host} />
                  </Field>
                )}
                <Field label="Port" htmlFor="ob-relay-port" error={errors.port}>
                  <Input
                    id="ob-relay-port"
                    inputMode="numeric"
                    value={port}
                    onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))}
                    className="max-w-[8rem] font-mono"
                    aria-invalid={!!errors.port}
                  />
                </Field>
                {provider && !provider.username && (
                  <Field label="SMTP username" htmlFor="ob-relay-user" error={errors.username}>
                    <Input id="ob-relay-user" value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="off" spellCheck={false} aria-invalid={!!errors.username} />
                  </Field>
                )}
                <Field
                  label={provider?.secretLabel ?? "Password"}
                  htmlFor="ob-relay-pass"
                  hint={savedPassword ? "Saved. Leave empty to keep it." : "Write-only: sealed on the server, never shown again."}
                  error={errors.password}
                >
                  <PasswordInput
                    id="ob-relay-pass"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    autoComplete="new-password"
                    placeholder={savedPassword ? "••••••••" : undefined}
                    aria-invalid={!!errors.password}
                  />
                </Field>
              </div>
            )}
          </CardBody>
          {info && (
            <CardFooter className="justify-between gap-y-1.5 text-xs leading-relaxed text-muted-foreground">
              <span>The provider's own DNS records (DKIM, bounce domain) are added under Sending once setup is done.</span>
              {provider?.docsUrl && (
                <a
                  href={provider.docsUrl}
                  target="_blank"
                  rel="noreferrer noopener"
                  className="inline-flex items-center gap-1 rounded-sm font-medium text-foreground/80 underline underline-offset-2 transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
                >
                  Where {provider.label} shows these <ExternalLink aria-hidden className="h-3 w-3" />
                  <span className="sr-only">(opens in a new tab)</span>
                </a>
              )}
            </CardFooter>
          )}
        </Card>
      )}

      <FormError>{errors.form}</FormError>
      <StepActions>
        <Button type="submit" variant="torii" loading={busy} disabled={mode === "relay" && !info} className="w-full sm:w-auto">
          {busy ? "Saving…" : "Continue"}
          {!busy && <ArrowRight aria-hidden className="h-3.5 w-3.5" />}
        </Button>
      </StepActions>
    </form>
  );
}

// ── 3. host record ───────────────────────────────────────────────────────────

function HostStep({ state, onState }: { state: OnboardingState; onState: (s: OnboardingState) => void }) {
  const step = stepOf(state, "host_record");
  const check = useCheck(onState);
  const domain = usePrimaryDomain(state);
  const host = state.mailHostname;
  const done = isDone(step?.status);
  return (
    <div className="space-y-5">
      <Detail step={step} />
      {!done && (
        <>
          <Card padded={false}>
            <CardHeader icon={Globe} title="Add this record" description="At the company that hosts your domain's DNS." />
            <CardBody className="space-y-4">
              <div className="grid gap-2 sm:grid-cols-[6.5rem_minmax(0,1fr)_minmax(0,1fr)]">
                <CopyField label="Type" value={state.server.ipv4 ? "A" : "AAAA"} />
                <CopyField label="Name" value={host} />
                <CopyField label="Value" value={state.server.ipv4 ?? state.server.ipv6 ?? ""} />
              </div>
              <ul className="space-y-1.5 text-xs leading-relaxed text-muted-foreground">
                {state.server.ipv4 && state.server.ipv6 && (
                  <li>
                    Optional: an AAAA record for <span className="font-mono text-foreground/85">{host}</span> →{" "}
                    <span className="font-mono text-foreground/85">{state.server.ipv6}</span>, but only if this server's IPv6 address also gets reverse DNS.
                  </li>
                )}
                <li>Using Cloudflare? Set it to “DNS only” (grey cloud): mail needs a direct connection. New records usually show up within minutes.</li>
              </ul>
            </CardBody>
          </Card>
          {domain && (
            <Disclosure icon={CloudCog} title="Connect Cloudflare and publish automatically" description={CLOUDFLARE_HINT}>
              <DnsProviderCard domain={domain} onChecked={() => void check()} />
            </Disclosure>
          )}
          <Recheck onCheck={check} />
        </>
      )}
    </div>
  );
}

// ── 4. certificate ───────────────────────────────────────────────────────────

function CertificateStep({ state, onRecheck }: { state: OnboardingState; onRecheck: () => Promise<void> }) {
  const step = stepOf(state, "certificate");
  const done = isDone(step?.status);
  const { cert, busy, tryNow } = useCertificate({ live: !done, onIssued: () => void onRecheck() });
  if (done) return <Detail step={step} />;
  const problem = !!cert?.problem;
  return (
    <Card padded={false}>
      <CardHeader
        icon={ShieldCheck}
        title="Let's Encrypt"
        description={
          <>
            Let's Encrypt checks over port 80 that <span className="font-mono text-[12.5px] text-foreground/85">{state.mailHostname}</span> reaches this server,
            then issues its certificate
            {state.panel.host !== state.mailHostname ? (
              <>
                {" "}
                (the console's address, <span className="font-mono text-[12.5px] text-foreground/85">{state.panel.host}</span>, stays on it)
              </>
            ) : null}
            . It renews by itself.
          </>
        }
      />
      <CardBody>
        {problem ? (
          <div className="rounded-lg border border-warning/30 bg-warning/[0.06] p-4">
            <CertificateProblem cert={cert} busy={busy} onTryNow={() => void tryNow()} fallback={step?.detail} />
          </div>
        ) : (
          <p className="flex items-center gap-3 rounded-lg border border-border bg-muted/35 px-4 py-3 text-[13px] leading-relaxed text-foreground/85" role="status">
            <Loader2 aria-hidden className="h-4 w-4 shrink-0 animate-spin text-muted-foreground motion-reduce:animate-none" />
            <span className="min-w-0 [overflow-wrap:anywhere]">{step?.detail ?? "Requesting the certificate…"}</span>
          </p>
        )}
      </CardBody>
    </Card>
  );
}

// ── 5. DNS records ───────────────────────────────────────────────────────────

function DnsStep({ state, onState }: { state: OnboardingState; onState: (s: OnboardingState) => void }) {
  const step = stepOf(state, "dns");
  const domain = usePrimaryDomain(state);
  const id = state.primaryDomainId;
  const [records, setRecords] = useState<DnsRecord[] | null>(null);
  const [zone, setZone] = useState("");
  const check = useCheck(onState);

  const refresh = useCallback(
    async (fresh: boolean) => {
      if (!id) return;
      try {
        const dns = fresh ? await api.checkDns(id) : await api.dns(id);
        setRecords(dns.records);
        setZone(dns.zoneFile);
      } catch {
        setRecords((r) => r ?? []);
      }
    },
    [id],
  );
  useEffect(() => {
    void refresh(false);
  }, [refresh]);
  // The records re-checked every minute, like the step itself.
  useEffect(() => {
    const t = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh(true);
    }, 60_000);
    return () => window.clearInterval(t);
  }, [refresh]);

  const required = useMemo(() => (records ?? []).filter((r) => r.required && r.id !== "ptr"), [records]);
  const optional = useMemo(() => (records ?? []).filter((r) => !r.required && r.status !== "pass"), [records]);
  const missing = required.filter((r) => r.status !== "pass");
  const found = required.length - missing.length;

  return (
    <div className="space-y-5">
      <Detail step={step} />
      {domain && !isDone(step?.status) && (
        <Disclosure icon={CloudCog} title="Connect Cloudflare and publish automatically" description={CLOUDFLARE_HINT}>
          <DnsProviderCard
            domain={domain}
            onChecked={(dns) => {
              setRecords(dns.records);
              void check();
            }}
          />
        </Disclosure>
      )}
      <Card padded={false}>
        <CardHeader
          title="Required records"
          description={records === null ? "Reading the records…" : `${found} of ${required.length} found in public DNS.`}
          actions={zone ? <CopyButton value={zone} label="zone file" showText text="Copy all as a zone file" /> : undefined}
          divider
        />
        {records === null ? (
          <div className="p-5">
            <Skeleton className="h-40" />
          </div>
        ) : (
          <ul className="divide-y divide-border/60">
            {required.map((r) => (
              <RecordRow key={r.id} r={r} />
            ))}
          </ul>
        )}
        {records !== null && missing.length === 0 && !isDone(step?.status) && (
          <CardFooter className="text-xs text-muted-foreground">Every required record resolves; the next check moves on.</CardFooter>
        )}
      </Card>
      {optional.length > 0 && (
        <details className="group rounded-xl border border-border bg-card/50">
          <summary className="flex cursor-pointer list-none items-center gap-2 rounded-xl px-4 py-3 text-[13px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 [&::-webkit-details-marker]:hidden">
            <span className="flex-1">
              {optional.length} optional record{optional.length === 1 ? "" : "s"}: setup does not wait for them
            </span>
            <ChevronDown aria-hidden className="h-3.5 w-3.5 transition-transform group-open:rotate-180 motion-reduce:transition-none" />
          </summary>
          <ul className="divide-y divide-border/60 border-t border-border/70">
            {optional.map((r) => (
              <RecordRow key={r.id} r={r} withPurpose />
            ))}
          </ul>
        </details>
      )}
      {!isDone(step?.status) && (
        <Recheck
          onCheck={async () => {
            await refresh(true);
            await check();
          }}
        />
      )}
    </div>
  );
}

function RecordRow({ r, withPurpose }: { r: DnsRecord; withPurpose?: boolean }) {
  const ok = r.status === "pass";
  const badge = (
    <StatusBadge size="xs" tone={ok ? "ok" : r.required ? "warn" : "muted"}>
      {ok ? "Found" : "Missing"}
    </StatusBadge>
  );
  const copy = <span className="-my-0.5 block w-7 shrink-0">{!ok && <CopyButton value={r.expected} label={`${r.type} value for ${r.name}`} />}</span>;
  return (
    <li className="px-4 py-3 sm:flex sm:items-start sm:gap-3 sm:px-5">
      {/* Phones: type, status and copy on one line, the record full width under it. */}
      <div className="flex items-center justify-between gap-2 sm:contents">
        <Chip className="w-14 shrink-0 justify-center sm:mt-px">{r.type}</Chip>
        <span className="flex items-center gap-1.5 sm:hidden">
          {badge}
          {copy}
        </span>
      </div>
      <div className="mt-2 min-w-0 sm:mt-0 sm:flex-1">
        <p className="break-all font-mono text-[13px] text-foreground">{r.name}</p>
        <p className="mt-0.5 line-clamp-2 break-all font-mono text-xs text-muted-foreground" title={r.expected}>
          {r.expected}
        </p>
        {withPurpose && <p className="mt-1 text-xs leading-relaxed text-muted-foreground">Without it: {r.purpose}</p>}
      </div>
      <span className="hidden shrink-0 pt-px sm:block">{badge}</span>
      <span className="hidden sm:block">{copy}</span>
    </li>
  );
}

// ── 6. reverse DNS ───────────────────────────────────────────────────────────

function PtrStep({ state, onState, onRelay }: { state: OnboardingState; onState: (s: OnboardingState) => void; onRelay: () => void }) {
  const step = stepOf(state, "ptr");
  const check = useCheck(onState);
  const ip = state.server.ipv4 ?? state.server.ipv6 ?? "";
  const [busy, setBusy] = useState(false);
  const relayed = state.sending.mode === "relay";

  async function skip(v: boolean) {
    setBusy(true);
    try {
      onState(await api.skipPtr(v));
      toast.info(v ? "Reverse DNS skipped for now. The overview keeps checking it." : "Reverse DNS is checked again.");
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  if (isDone(step?.status) && step?.status === "skipped" && state.ptrSkipped) {
    return (
      <div className="space-y-5">
        <Detail step={step} />
        <StepActions>
          <Button variant="outline" onClick={() => void skip(false)} disabled={busy} className="w-full sm:w-auto">
            Check reverse DNS instead
          </Button>
        </StepActions>
      </div>
    );
  }
  if (isDone(step?.status)) return <Detail step={step} />;
  return (
    <div className="space-y-5">
      <Card padded={false}>
        <CardHeader
          icon={Server}
          title="Set it at your VPS provider"
          description={
            <>
              Receiving servers check that this server's address names it back. This is set in your <strong className="font-medium text-foreground/90">VPS provider's</strong>{" "}
              control panel (often “Networking” or “Reverse DNS”), not at your DNS host.
            </>
          }
        />
        <CardBody>
          <div className="grid gap-2 sm:grid-cols-2">
            <CopyField label="IP address" value={ip} />
            <CopyField label="Reverse DNS (PTR) name" value={state.mailHostname} />
          </div>
        </CardBody>
      </Card>
      <Detail step={step} />
      {!relayed && (
        <Callout
          tone="muted"
          title="Can't set it now?"
          action={
            <>
              <Button size="sm" variant="outline" onClick={() => void skip(true)} disabled={busy}>
                Skip for now
              </Button>
              <Button size="sm" variant="ghost" onClick={onRelay}>
                Use a relay instead
              </Button>
            </>
          }
        >
          Skip for now: receiving works, but Gmail and Outlook will likely reject mail sent directly from this server until reverse DNS is set. Or send
          through a relay provider, which doesn't need it.
        </Callout>
      )}
      <Recheck onCheck={check} />
    </div>
  );
}

// ── 7. first mailbox ─────────────────────────────────────────────────────────

function MailboxStep({ state, onCreated }: { state: OnboardingState; onCreated: (r: FirstMailboxResponse) => Promise<void> }) {
  const step = stepOf(state, "first_mailbox");
  const [local, setLocal] = useState("hello");
  const [name, setName] = useState("");
  const [invite, setInvite] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (isDone(step?.status)) return <Detail step={step} />;

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
      const r = await api.createFirstMailbox({ local: v, displayName: name.trim() || undefined, invite });
      toast.success(`${r.mailbox.address} is ready.`);
      await onCreated(r);
    } catch (err) {
      setError(err instanceof ApiRequestError && err.field === "local" ? err.message : errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={onSubmit} noValidate className="space-y-5">
      <Card padded={false}>
        <CardBody className="space-y-6 pt-5">
          <div className="grid gap-x-4 gap-y-5 sm:grid-cols-2">
            <Field label="Address" htmlFor="ob-local" error={error}>
              <div className="flex items-stretch">
                <Input
                  id="ob-local"
                  value={local}
                  onChange={(e) => setLocal(e.target.value)}
                  autoComplete="off"
                  autoCapitalize="none"
                  spellCheck={false}
                  className="min-w-0 flex-1 rounded-r-none font-mono"
                  aria-invalid={!!error}
                  aria-describedby={error ? "ob-local-err" : "ob-local-domain"}
                />
                <span
                  id="ob-local-domain"
                  className="grid max-w-[60%] shrink-0 place-items-center truncate rounded-r-lg border border-l-0 border-input bg-muted/60 px-3 font-mono text-[13px] text-muted-foreground"
                >
                  @{state.primaryDomain}
                </span>
              </div>
            </Field>
            <Field label="Display name" optional htmlFor="ob-name">
              <Input id="ob-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Front desk" />
            </Field>
          </div>
          <fieldset>
            <legend className="mb-2">
              <Label>Password</Label>
            </legend>
            <div className="grid gap-3 sm:grid-cols-2">
              <RadioCard
                compact
                name="ob-pw"
                value="invite"
                checked={invite}
                onChange={() => setInvite(true)}
                title="Send an invite link"
                description="Whoever opens it chooses the password (one use, 7 days). Nobody else ever sees it."
                className="h-full"
              />
              <RadioCard
                compact
                name="ob-pw"
                value="password"
                checked={!invite}
                onChange={() => setInvite(false)}
                title="Show me a strong password"
                description="Shown once here; the server keeps no readable copy."
                className="h-full"
              />
            </div>
          </fieldset>
        </CardBody>
      </Card>
      <StepActions>
        <Button type="submit" variant="torii" loading={busy} className="w-full sm:w-auto">
          {busy ? "Creating…" : "Create mailbox"}
        </Button>
      </StepActions>
    </form>
  );
}

// ── 8. done ──────────────────────────────────────────────────────────────────

function DoneStep({ state, created, onFinish }: { state: OnboardingState; created: FirstMailboxResponse | null; onFinish: () => Promise<void> }) {
  const moved = state.panel.host !== state.mailHostname;
  return (
    <div className="space-y-5">
      {!state.complete && (
        <p className="flex items-center gap-2 text-[13px] text-muted-foreground" role="status">
          <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" /> Finishing…
        </p>
      )}
      <SetupComplete created={created} finished={state.complete} onDismiss={() => void onFinish()} dismissLabel="Go to the overview" className="mb-0" />
      {moved && (
        <Callout tone="info" title="Bookmark the console's new address">
          The console now also opens at{" "}
          <a
            className="rounded-sm font-mono text-foreground underline underline-offset-2 [overflow-wrap:anywhere] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
            href={`https://${state.mailHostname}/console`}
          >
            https://{state.mailHostname}/console
          </a>
          , with the trusted certificate. Bookmark that one.
        </Callout>
      )}
    </div>
  );
}
