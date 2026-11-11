import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Link, Navigate, useNavigate } from "react-router-dom";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  ChevronRight,
  CloudCog,
  ExternalLink,
  Hourglass,
  Loader2,
  Minus,
  RefreshCw,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Input } from "@/design/vendor/ui/input";
import { Card, Field, SectionLabel, Skeleton, selectClass } from "@/design/vendor/admin/ui";
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
import { FieldError, LoadError, RadioRow, usePageTitle } from "@/components/kit";
import { PasswordInput } from "@/components/password-input";
import { CheckRow, StatusBadge, toneText, type Tone } from "@/components/status";
import { ThemeToggle } from "@/components/theme-toggle";
import { YunoriMark } from "@/components/yunori-logo";
import { toast } from "@/lib/toast";
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
        <div aria-busy="true" className="grid gap-6 lg:grid-cols-[15rem_minmax(0,1fr)]">
          <Skeleton className="h-80" />
          <Skeleton className="h-96" />
        </div>
      </Frame>
    );
  }

  if (!state.terminalDone) {
    return (
      <Frame>
        <Card>
          <SectionLabel>Not yet</SectionLabel>
          <h1 className="-mt-1 font-serif text-[24px] tracking-tight">The terminal part has not finished</h1>
          <p className="mt-2 max-w-xl text-sm leading-relaxed text-muted-foreground">
            Run <code className="font-mono text-foreground/85">sudo yunori-mail install</code> on the server again: it finishes installing the mail engine,
            then this page continues.
          </p>
        </Card>
      </Frame>
    );
  }

  const meta = WIZARD.find((w) => w.id === shown)!;
  const index = WIZARD.findIndex((w) => w.id === shown);
  const step = stepOf(state, shown);

  return (
    <Frame panel={state.panel.host}>
      <div className="grid gap-6 lg:grid-cols-[15rem_minmax(0,1fr)] lg:gap-10">
        <Stepper state={state} current={current} shown={shown} onOpen={(id) => setViewing(id === current ? null : id)} />

        <section aria-labelledby="onboarding-step-title" className="min-w-0">
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
            <p className="text-[11px] tabular-nums text-muted-foreground">
              Step {index + 1} of {WIZARD.length}
            </p>
            {step && shown !== "done" && <StepBadge status={step.status} current={shown === current} />}
          </div>
          <h1
            id="onboarding-step-title"
            ref={headingRef}
            tabIndex={-1}
            className="mt-2 font-serif text-[26px] leading-tight tracking-tight focus:outline-none sm:text-[30px]"
          >
            {meta.title}
          </h1>
          <p className="mt-1.5 max-w-2xl text-sm leading-relaxed text-muted-foreground">{meta.lead}</p>

          {viewing && viewing !== current && (
            <button
              type="button"
              onClick={() => setViewing(null)}
              className="mt-3 inline-flex items-center gap-1 rounded-sm text-xs text-muted-foreground underline-offset-2 transition-colors hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <ArrowLeft aria-hidden className="h-3 w-3" /> Back to “{WIZARD.find((w) => w.id === current)?.title}”
            </button>
          )}

          <div className="mt-6 max-w-3xl">
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
      <header className="border-b border-border/50">
        <div className="mx-auto flex max-w-6xl items-center gap-3 px-5 py-3 sm:px-8">
          <YunoriMark className="h-6 w-6" />
          <p className="text-sm font-medium tracking-tight">
            Set up Yunori Mail
            {panel && <span className="ml-2 hidden font-mono text-[12px] font-normal text-muted-foreground sm:inline">{panel}</span>}
          </p>
          <div className="ml-auto flex items-center gap-1">
            <Link
              to="/console"
              className="rounded-sm px-2 py-1 text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              Console
            </Link>
            <ThemeToggle />
          </div>
        </div>
      </header>
      <main className="mx-auto max-w-6xl px-5 py-8 sm:px-8 sm:py-10">{children}</main>
    </div>
  );
}

const ICON: Record<StepStatus, LucideIcon | null> = {
  done: Check,
  skipped: Minus,
  running: Loader2,
  waiting: Hourglass,
  failed: Hourglass,
  pending: null,
};

function StepBadge({ status, current }: { status: StepStatus; current: boolean }) {
  const m: Record<StepStatus, { label: string; tone: Tone }> = {
    done: { label: "Done", tone: "ok" },
    skipped: { label: "Skipped for now", tone: "muted" },
    running: { label: "Checking", tone: "muted" },
    waiting: { label: current ? "Your turn" : "Waiting", tone: "warn" },
    failed: { label: "Needs attention", tone: "bad" },
    pending: { label: current ? "Your turn" : "Up next", tone: current ? "warn" : "muted" },
  };
  const { label, tone } = m[status];
  return (
    <StatusBadge tone={tone} icon={status === "done" ? Check : status === "waiting" ? Hourglass : undefined}>
      {label}
    </StatusBadge>
  );
}

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
  return (
    <nav aria-label="Setup steps" className="min-w-0">
      <p className="text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
        {doneCount} of {total} done
      </p>
      <div className="mt-2 h-1 overflow-hidden rounded-full bg-foreground/10" aria-hidden>
        <div className="h-full rounded-full bg-torii transition-[width] motion-reduce:transition-none" style={{ width: `${(doneCount / total) * 100}%` }} />
      </div>
      <ol className="mt-4 flex gap-1 overflow-x-auto pb-1 lg:block lg:space-y-0.5 lg:overflow-visible">
        {WIZARD.map((w, i) => {
          const st = w.id === "done" ? (state.complete ? "done" : "pending") : stepOf(state, w.id)?.status ?? "pending";
          const reachable = isDone(st) || w.id === current;
          const Icon = ICON[st];
          const active = w.id === shown;
          return (
            <li key={w.id} className="shrink-0">
              <button
                type="button"
                disabled={!reachable}
                onClick={() => onOpen(w.id)}
                aria-current={active ? "step" : undefined}
                className={cn(
                  "relative flex w-full items-center gap-2.5 rounded-md py-1.5 pl-2.5 pr-2.5 text-left text-[13px] leading-snug transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                  active ? "bg-foreground/[0.06] font-medium text-foreground" : reachable ? "text-foreground/75 hover:bg-foreground/[0.04]" : "text-muted-foreground",
                  !reachable && "cursor-default",
                )}
              >
                {active && <span aria-hidden className="absolute inset-y-1.5 left-0 hidden w-0.5 rounded-full bg-torii lg:block" />}
                <span
                  aria-hidden
                  className={cn(
                    "grid h-4 w-4 shrink-0 place-items-center rounded-full text-[9px] tabular-nums",
                    st === "done" && "bg-foreground/80 text-background",
                    st === "skipped" && "bg-foreground/15 text-foreground/70",
                    st === "pending" && "border border-foreground/20",
                    (st === "waiting" || st === "running" || st === "failed") && "text-torii",
                  )}
                >
                  {Icon ? <Icon className={cn("h-3 w-3", st === "running" && "animate-spin motion-reduce:animate-none")} strokeWidth={2.5} /> : i + 1}
                </span>
                <span className="whitespace-nowrap lg:whitespace-normal">{w.title}</span>
                <span className="sr-only">: {isDone(st) ? (st === "skipped" ? "skipped" : "done") : w.id === current ? "current step" : "not yet"}</span>
              </button>
            </li>
          );
        })}
      </ol>
      <p className="mt-5 hidden text-[11px] leading-relaxed text-muted-foreground lg:block">
        You can close this page at any time. The server keeps checking by itself and picks up where you left off.
      </p>
    </nav>
  );
}

function SetupLog({ state }: { state: OnboardingState }) {
  const lines = state.logs.slice(-40);
  if (!lines.length) return null;
  return (
    <details className="group mt-10 max-w-3xl border-t border-border/50 pt-3">
      <summary className="flex cursor-pointer list-none items-center gap-1.5 rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
        <ChevronRight aria-hidden className="h-3.5 w-3.5 transition-transform group-open:rotate-90 motion-reduce:transition-none" />
        Setup log
      </summary>
      <ul className="mt-3 max-h-64 space-y-1 overflow-auto rounded-lg border border-border/60 bg-muted/30 p-3 font-mono text-[11px] leading-relaxed" tabIndex={0} aria-label="Setup log">
        {lines.map((l, i) => (
          <li key={i} className="flex gap-3">
            <time dateTime={l.at} className="shrink-0 text-muted-foreground">
              {new Date(l.at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}
            </time>
            <span className={cn("min-w-0 break-words", l.level === "info" ? "text-foreground/85" : toneText(l.level === "warn" ? "warn" : "bad"))}>
              {l.message}
            </span>
          </li>
        ))}
      </ul>
    </details>
  );
}

// ── shared bits ──────────────────────────────────────────────────────────────

function CopyField({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="min-w-0 rounded-lg border border-border/60 bg-card px-3 py-2">
      <p className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground">{label}</p>
      <div className="mt-0.5 flex items-center justify-between gap-2">
        <code className="min-w-0 break-all font-mono text-[13px] text-foreground">{value}</code>
        <CopyButton value={value} label={label} />
      </div>
      {hint && <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">{hint}</p>}
    </div>
  );
}

/** The server's own words for a step (what it found, what to do). */
function Detail({ step }: { step: InstallStep | undefined }) {
  if (!step?.detail) return null;
  const done = isDone(step.status);
  return (
    <p
      className={cn(
        "rounded-lg border px-3 py-2.5 text-sm leading-relaxed [overflow-wrap:anywhere]",
        done ? "border-border/60 bg-muted/30 text-foreground/85" : "border-torii/30 bg-torii/[0.04] text-foreground",
      )}
      role={done ? undefined : "status"}
    >
      {step.detail}
    </p>
  );
}

/** "Check now" plus a note that the server re-checks every minute anyway. */
function Recheck({ onCheck, label = "Check now" }: { onCheck: () => Promise<void>; label?: string }) {
  const [busy, setBusy] = useState(false);
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
      <Button
        size="sm"
        onClick={async () => {
          setBusy(true);
          try {
            await onCheck();
          } finally {
            setBusy(false);
          }
        }}
        disabled={busy}
      >
        <RefreshCw aria-hidden className={cn("h-3.5 w-3.5", busy && "animate-spin motion-reduce:animate-none")} />
        {busy ? "Checking…" : label}
      </Button>
      <span className="text-[11px] text-muted-foreground">Checked against public DNS, again every minute by itself.</span>
    </div>
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

function Disclosure({ icon: Icon, title, children }: { icon: LucideIcon; title: string; children: ReactNode }) {
  return (
    <details className="group rounded-lg border border-border/60 bg-card/40">
      <summary className="flex cursor-pointer list-none items-center gap-2 rounded-lg px-3 py-2.5 text-sm text-foreground/90 transition-colors hover:bg-foreground/[0.03] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
        <Icon aria-hidden className="h-4 w-4 text-muted-foreground" strokeWidth={1.75} />
        <span className="min-w-0 flex-1">{title}</span>
        <ChevronRight aria-hidden className="h-3.5 w-3.5 text-muted-foreground transition-transform group-open:rotate-90 motion-reduce:transition-none" />
      </summary>
      <div className="border-t border-border/50 p-3">{children}</div>
    </details>
  );
}

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
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="Mail domain" htmlFor="ob-domain" hint="The part after @ in every address.">
          <Input
            id="ob-domain"
            value={domain}
            onChange={(e) => changeDomain(e.target.value)}
            placeholder="example.com"
            autoComplete="off"
            spellCheck={false}
            autoFocus={!state.identity}
            className="font-mono"
            aria-invalid={!!errors.domain}
            aria-describedby={errors.domain ? "ob-domain-err" : undefined}
          />
          <FieldError id="ob-domain-err">{errors.domain}</FieldError>
          {parent && !errors.domain && (
            <p className="text-[11px] leading-relaxed text-foreground/85">
              Addresses would look like you@{d}.{" "}
              <button type="button" className="underline underline-offset-2" onClick={() => changeDomain(parent)}>
                Use {parent} instead
              </button>
            </p>
          )}
        </Field>
        <Field label="Mail server hostname" htmlFor="ob-host" hint="This server's own name: where mail is delivered and mail apps connect.">
          <Input
            id="ob-host"
            value={hostname}
            onChange={(e) => {
              setHostEdited(true);
              setHostname(e.target.value);
            }}
            placeholder="mail.example.com"
            autoComplete="off"
            spellCheck={false}
            className="font-mono"
            aria-invalid={!!errors.hostname}
            aria-describedby={errors.hostname ? "ob-host-err" : undefined}
          />
          <FieldError id="ob-host-err">{errors.hostname}</FieldError>
        </Field>
      </div>
      <Field
        label="Your email on another provider"
        htmlFor="ob-email"
        hint="For certificate notices and alerts. It must not depend on this server: if the server is down, you still hear about it."
      >
        <Input
          id="ob-email"
          type="email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@gmail.com"
          autoComplete="email"
          className="max-w-md"
          aria-invalid={!!errors.adminEmail}
          aria-describedby={errors.adminEmail ? "ob-email-err" : undefined}
        />
        <FieldError id="ob-email-err">{errors.adminEmail}</FieldError>
      </Field>

      {d && /\./.test(d) && (
        <p className="rounded-lg border border-border/60 bg-muted/30 px-3 py-2.5 text-xs leading-relaxed text-muted-foreground">
          Addresses look like <span className="font-mono text-foreground/85">you@{d}</span>. Other mail servers deliver to{" "}
          <span className="font-mono text-foreground/85">{hostname.trim() || `mail.${d}`}</span>, and mail apps connect there.
        </p>
      )}
      {panelNote && <p className="text-xs leading-relaxed text-muted-foreground">{panelNote}</p>}
      {laterDone && (
        <p className="text-xs leading-relaxed text-foreground/85">Changing these checks the hostname, certificate and DNS steps again.</p>
      )}
      {errors.form && (
        <p role="alert" className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs leading-relaxed text-foreground/90">
          {errors.form}
        </p>
      )}
      <Button type="submit" variant="torii" disabled={busy}>
        {busy ? "Saving…" : state.identity ? "Save" : "Continue"}
        {!busy && <ArrowRight aria-hidden className="h-3.5 w-3.5" />}
      </Button>
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
  return (
    <form onSubmit={onSubmit} noValidate className="space-y-5">
      <ul className="rounded-lg border border-border/60 bg-card px-3">
        <CheckRow check={state.sending.port25} compact />
      </ul>
      <fieldset className="space-y-3">
        <legend className="sr-only">How this server sends mail</legend>
        <RadioRow
          name="ob-sending"
          checked={mode === "direct"}
          onChange={() => setMode("direct")}
          label={`Directly from this server${state.sending.suggested === "direct" ? " (suggested)" : ""}`}
          hint="Needs outbound port 25 and reverse DNS. Nothing else to sign up for."
        />
        <RadioRow
          name="ob-sending"
          checked={mode === "relay"}
          onChange={() => setMode("relay")}
          label={`Through a relay provider${state.sending.suggested === "relay" ? " (suggested)" : ""}`}
          hint={
            blocked
              ? "Your provider seems to block outbound port 25: a relay (Amazon SES, Resend, Postmark…) delivers for you. Receiving works either way."
              : "Amazon SES, Resend, Postmark, Mailgun… They deliver for you; reverse DNS becomes optional."
          }
        />
      </fieldset>

      {mode === "relay" && (
        <div className="space-y-4 rounded-lg border border-border/60 bg-card/40 p-4">
          {!info ? (
            <Skeleton className="h-24" />
          ) : (
            <>
              <div className="grid gap-4 sm:grid-cols-2">
                <Field label="Provider" htmlFor="ob-provider">
                  <select id="ob-provider" className={selectClass} value={providerKey} onChange={(e) => pickProvider(e.target.value)}>
                    {info.providers.map((p) => (
                      <option key={p.key} value={p.key}>
                        {p.label}
                      </option>
                    ))}
                  </select>
                  <FieldError id="ob-provider-err">{errors.provider}</FieldError>
                </Field>
                {provider && provider.regions.length > 0 && (
                  <Field label="Region" htmlFor="ob-region">
                    <select id="ob-region" className={selectClass} value={region} onChange={(e) => setRegion(e.target.value)}>
                      {provider.regions.map((r) => (
                        <option key={r} value={r}>
                          {r}
                        </option>
                      ))}
                    </select>
                    <FieldError id="ob-region-err">{errors.region}</FieldError>
                  </Field>
                )}
                {provider && !provider.hostTemplate && (
                  <Field label="SMTP host" htmlFor="ob-relay-host">
                    <Input id="ob-relay-host" value={host} onChange={(e) => setHost(e.target.value)} className="font-mono" spellCheck={false} aria-invalid={!!errors.host} />
                    <FieldError id="ob-relay-host-err">{errors.host}</FieldError>
                  </Field>
                )}
                <Field label="Port" htmlFor="ob-relay-port">
                  <Input id="ob-relay-port" inputMode="numeric" value={port} onChange={(e) => setPort(e.target.value.replace(/\D/g, ""))} className="max-w-[8rem] font-mono" aria-invalid={!!errors.port} />
                  <FieldError id="ob-relay-port-err">{errors.port}</FieldError>
                </Field>
                {provider && !provider.username && (
                  <Field label="SMTP username" htmlFor="ob-relay-user">
                    <Input id="ob-relay-user" value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="off" spellCheck={false} aria-invalid={!!errors.username} />
                    <FieldError id="ob-relay-user-err">{errors.username}</FieldError>
                  </Field>
                )}
                <Field
                  label={provider?.secretLabel ?? "Password"}
                  htmlFor="ob-relay-pass"
                  hint={savedPassword ? "Saved. Leave empty to keep it." : "Write-only: sealed on the server, never shown again."}
                >
                  <PasswordInput
                    id="ob-relay-pass"
                    value={password}
                    onChange={(e) => setPassword(e.target.value)}
                    autoComplete="new-password"
                    placeholder={savedPassword ? "••••••••" : undefined}
                    aria-invalid={!!errors.password}
                  />
                  <FieldError id="ob-relay-pass-err">{errors.password}</FieldError>
                </Field>
              </div>
              {provider?.docsUrl && (
                <a
                  href={provider.docsUrl}
                  target="_blank"
                  rel="noreferrer noopener"
                  className="inline-flex items-center gap-1 text-xs text-muted-foreground underline underline-offset-2 transition-colors hover:text-foreground"
                >
                  Where {provider.label} shows these <ExternalLink aria-hidden className="h-3 w-3" />
                  <span className="sr-only">(opens in a new tab)</span>
                </a>
              )}
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                The provider's own DNS records (DKIM, bounce domain) are added under Sending once setup is done.
              </p>
            </>
          )}
        </div>
      )}

      {errors.form && (
        <p role="alert" className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs leading-relaxed text-foreground/90">
          {errors.form}
        </p>
      )}
      <Button type="submit" variant="torii" disabled={busy || (mode === "relay" && !info)}>
        {busy ? "Saving…" : "Continue"}
        {!busy && <ArrowRight aria-hidden className="h-3.5 w-3.5" />}
      </Button>
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
          <p className="text-sm leading-relaxed text-foreground/90">At the company that hosts your domain's DNS, add:</p>
          <div className="grid gap-2 sm:grid-cols-3">
            <CopyField label="Type" value={state.server.ipv4 ? "A" : "AAAA"} />
            <CopyField label="Name" value={host} />
            <CopyField label="Value" value={state.server.ipv4 ?? state.server.ipv6 ?? ""} />
          </div>
          {state.server.ipv4 && state.server.ipv6 && (
            <p className="text-xs leading-relaxed text-muted-foreground">
              Optional: an AAAA record for <span className="font-mono">{host}</span> → <span className="font-mono">{state.server.ipv6}</span>, but only if
              this server's IPv6 address also gets reverse DNS.
            </p>
          )}
          <p className="text-xs leading-relaxed text-muted-foreground">
            Using Cloudflare? Set it to “DNS only” (grey cloud): mail needs a direct connection. New records usually show up within minutes.
          </p>
          <Recheck onCheck={check} />
          {domain && (
            <Disclosure icon={CloudCog} title="Connect Cloudflare and publish automatically">
              <DnsProviderCard domain={domain} onChecked={() => void check()} />
            </Disclosure>
          )}
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
    <div className="space-y-4">
      <p className="text-sm leading-relaxed text-foreground/90">
        Let's Encrypt checks over port 80 that <span className="font-mono text-[13px]">{state.mailHostname}</span> reaches this server, then issues its
        certificate{state.panel.host !== state.mailHostname ? <> (the console's address, <span className="font-mono text-[13px]">{state.panel.host}</span>, stays on it)</> : null}. It renews by itself.
      </p>
      {problem ? (
        <CertificateProblem cert={cert} busy={busy} onTryNow={() => void tryNow()} fallback={step?.detail} />
      ) : (
        <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
          <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" />
          {step?.detail ?? "Requesting the certificate…"}
        </p>
      )}
    </div>
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

  return (
    <div className="space-y-5">
      <Detail step={step} />
      {domain && !isDone(step?.status) && (
        <Disclosure icon={CloudCog} title="Connect Cloudflare and publish automatically">
          <DnsProviderCard
            domain={domain}
            onChecked={(dns) => {
              setRecords(dns.records);
              void check();
            }}
          />
        </Disclosure>
      )}
      <div>
        <div className="mb-2 flex flex-wrap items-baseline justify-between gap-2">
          <SectionLabel>Required records</SectionLabel>
          {zone && <CopyButton value={zone} label="zone file" showText text="Copy all as a zone file" />}
        </div>
        {records === null ? (
          <Skeleton className="h-40" />
        ) : (
          <ul className="divide-y divide-border/50 overflow-hidden rounded-lg border border-border/60 bg-card">
            {required.map((r) => (
              <RecordRow key={r.id} r={r} />
            ))}
          </ul>
        )}
        {records !== null && missing.length === 0 && !isDone(step?.status) && (
          <p className="mt-2 text-xs text-muted-foreground">Every required record resolves; the next check moves on.</p>
        )}
      </div>
      {optional.length > 0 && (
        <details className="group">
          <summary className="flex cursor-pointer list-none items-center gap-1.5 rounded-sm text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
            <ChevronRight aria-hidden className="h-3.5 w-3.5 transition-transform group-open:rotate-90 motion-reduce:transition-none" />
            {optional.length} optional record{optional.length === 1 ? "" : "s"}: setup does not wait for them
          </summary>
          <ul className="mt-2 divide-y divide-border/50 overflow-hidden rounded-lg border border-border/60 bg-card">
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
  return (
    <li className="flex items-start gap-3 px-3 py-2.5">
      <span className="mt-0.5 w-12 shrink-0 rounded border border-border/70 px-1 py-px text-center font-mono text-[10px] text-foreground/80">{r.type}</span>
      <div className="min-w-0 flex-1">
        <p className="break-all font-mono text-xs text-foreground/90">{r.name}</p>
        <p className="mt-0.5 line-clamp-2 break-all font-mono text-[11px] text-muted-foreground" title={r.expected}>
          {r.expected}
        </p>
        {withPurpose && <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">Without it: {r.purpose}</p>}
      </div>
      <span className={cn("mt-0.5 shrink-0 text-[11px]", ok ? toneText("ok") : toneText(r.required ? "warn" : "muted"))}>{ok ? "Found" : "Missing"}</span>
      {!ok && <CopyButton value={r.expected} label={`${r.type} value for ${r.name}`} />}
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
      <div className="space-y-4">
        <Detail step={step} />
        <Button size="sm" variant="outline" onClick={() => void skip(false)} disabled={busy}>
          Check reverse DNS instead
        </Button>
      </div>
    );
  }
  if (isDone(step?.status)) return <Detail step={step} />;
  return (
    <div className="space-y-5">
      <p className="text-sm leading-relaxed text-foreground/90">
        Receiving servers check that this server's address names it back. This is set in your <strong className="font-medium">VPS provider's</strong>{" "}
        control panel (often “Networking” or “Reverse DNS”), not at your DNS host.
      </p>
      <div className="grid gap-2 sm:grid-cols-2">
        <CopyField label="IP address" value={ip} />
        <CopyField label="Reverse DNS (PTR) name" value={state.mailHostname} />
      </div>
      <Detail step={step} />
      <Recheck onCheck={check} />
      {!relayed && (
        <div className="rounded-lg border border-border/60 bg-muted/30 p-3">
          <p className="text-xs leading-relaxed text-foreground/85">
            Can't set it now? Skip for now: receiving works, but Gmail and Outlook will likely reject mail sent directly from this server until reverse DNS
            is set. Or send through a relay provider, which doesn't need it.
          </p>
          <div className="mt-2.5 flex flex-wrap gap-2">
            <Button size="sm" variant="outline" onClick={() => void skip(true)} disabled={busy}>
              Skip for now
            </Button>
            <Button size="sm" variant="ghost" onClick={onRelay}>
              Use a relay instead
            </Button>
          </div>
        </div>
      )}
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
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="Address" htmlFor="ob-local">
          <div className="flex items-center rounded-md border border-input bg-card focus-within:ring-1 focus-within:ring-ring">
            <Input
              id="ob-local"
              value={local}
              onChange={(e) => setLocal(e.target.value)}
              autoComplete="off"
              spellCheck={false}
              className="border-0 bg-transparent font-mono shadow-none focus-visible:ring-0"
              aria-invalid={!!error}
              aria-describedby={error ? "ob-local-err" : undefined}
            />
            <span className="shrink-0 pr-3 font-mono text-[13px] text-muted-foreground">@{state.primaryDomain}</span>
          </div>
          <FieldError id="ob-local-err">{error}</FieldError>
        </Field>
        <Field label="Display name (optional)" htmlFor="ob-name">
          <Input id="ob-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Front desk" />
        </Field>
      </div>
      <fieldset className="space-y-3">
        <legend className="mb-1 text-xs font-medium text-foreground/80">Password</legend>
        <RadioRow
          name="ob-pw"
          checked={invite}
          onChange={() => setInvite(true)}
          label="Send an invite link"
          hint="Whoever opens it chooses the password (one use, 7 days). Nobody else ever sees it."
        />
        <RadioRow name="ob-pw" checked={!invite} onChange={() => setInvite(false)} label="Show me a strong password" hint="Shown once here; the server keeps no readable copy." />
      </fieldset>
      <Button type="submit" variant="torii" disabled={busy}>
        {busy ? "Creating…" : "Create mailbox"}
      </Button>
    </form>
  );
}

// ── 8. done ──────────────────────────────────────────────────────────────────

function DoneStep({ state, created, onFinish }: { state: OnboardingState; created: FirstMailboxResponse | null; onFinish: () => Promise<void> }) {
  const moved = state.panel.host !== state.mailHostname;
  return (
    <div className="space-y-4">
      {!state.complete && (
        <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
          <Loader2 aria-hidden className="h-4 w-4 animate-spin motion-reduce:animate-none" /> Finishing…
        </p>
      )}
      <SetupComplete created={created} finished={state.complete} onDismiss={() => void onFinish()} dismissLabel="Go to the overview" />
      {moved && (
        <p className="text-xs leading-relaxed text-muted-foreground">
          The console now also opens at{" "}
          <a className="font-mono text-foreground/85 underline underline-offset-2" href={`https://${state.mailHostname}/console`}>
            https://{state.mailHostname}/console
          </a>
          , with the trusted certificate. Bookmark that one.
        </p>
      )}
    </div>
  );
}
