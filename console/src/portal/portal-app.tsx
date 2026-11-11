import { useEffect, useRef, useState, type ReactNode } from "react";
import { Route, Routes, useNavigate, useSearchParams } from "react-router-dom";
import { ArrowDownToLine, KeyRound, Lock, LogOut, Plus, Reply, Settings2, Smartphone, Trash2, type LucideIcon } from "lucide-react";
import { YunoriMark } from "@/components/yunori-logo";
import { Button } from "@/ui/button";
import { Input, Label } from "@/ui/form";
import { Avatar } from "@/ui/badge";
import { RingMeter } from "@/ui/data-viz";
import { Callout, Skeleton } from "@/ui/feedback";
import { ApiRequestError, errorMessage, portalApi } from "@/api/client";
import type { AppPassword, PortalMe } from "@/api/types";
import { AuthFrame, FormError } from "@/components/auth-frame";
import { ConfirmAction } from "@/components/confirm";
import { CopyButton } from "@/components/copy-button";
import { FieldError, LoadError, usePageTitle } from "@/components/kit";
import { PasswordInput, StrengthHint } from "@/components/password-input";
import { SecretReveal } from "@/components/secret-reveal";
import { ThemeToggle } from "@/components/theme-toggle";
import { fmtBytes, fmtDate, pct } from "@/lib/format";
import { MIN_PASSWORD } from "@/lib/password";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";
import { NotFound } from "@/pages/not-found";
import { ConnectDevices } from "./devices";
import { MoveMail } from "./migrate";
import { MailRulesCard } from "./rules";
import { PortalWelcome } from "./welcome";

/**
 * /account — the self-service portal every mailbox user gets. Separate session
 * (cookie `yunori_portal`), same design tokens, built for phones first.
 */
export default function PortalApp() {
  return (
    <Routes>
      <Route path="login" element={<PortalLogin />} />
      <Route path="welcome" element={<PortalWelcome />} />
      <Route index element={<PortalHome />} />
      <Route path="*" element={<NotFound />} />
    </Routes>
  );
}

function safePortalNext(raw: string | null): string {
  return raw && raw.startsWith("/account") && !raw.startsWith("//") && !raw.startsWith("/account/login") ? raw : "/account";
}

// ── Sign in ──────────────────────────────────────────────────────────────────

function PortalLogin() {
  usePageTitle("Sign in to your mailbox");
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const next = safePortalNext(params.get("next"));
  const [checking, setChecking] = useState(true);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Already signed in? Skip the form.
  useEffect(() => {
    let cancelled = false;
    portalApi
      .probe()
      .then(() => !cancelled && navigate(next, { replace: true }))
      .catch(() => !cancelled && setChecking(false));
    return () => {
      cancelled = true;
    };
  }, [navigate, next]);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (!email.trim() || !password) {
      setError("Enter your email address and password.");
      return;
    }
    setBusy(true);
    try {
      await portalApi.login({ email: email.trim(), password });
      navigate(next, { replace: true });
    } catch (err) {
      setPassword("");
      setError(
        err instanceof ApiRequestError && err.status === 401
          ? "That address and password didn't match. Use the mailbox password your administrator gave you."
          : errorMessage(err),
      );
    } finally {
      setBusy(false);
    }
  }

  if (checking) return <div className="min-h-dvh bg-background" aria-busy="true" />;

  return (
    <AuthFrame
      eyebrow="Yunori Mail"
      title="Your mailbox"
      subtitle="Sign in to set up your devices, change your password and manage app passwords."
      footnote="Forgot your password? Ask the person who runs this mail server to reset it."
    >
      <form onSubmit={onSubmit} className="space-y-4" noValidate>
        <div className="space-y-1.5">
          <Label htmlFor="p-email">Email address</Label>
          <Input id="p-email" type="email" autoComplete="username" autoFocus required value={email} onChange={(e) => setEmail(e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="p-password">Password</Label>
          <PasswordInput id="p-password" autoComplete="current-password" required value={password} onChange={(e) => setPassword(e.target.value)} />
        </div>
        <FormError>{error}</FormError>
        <Button type="submit" variant="torii" size="lg" className="w-full" loading={busy}>
          {busy ? "Signing in…" : "Sign in"}
        </Button>
      </form>
    </AuthFrame>
  );
}

// ── Home ─────────────────────────────────────────────────────────────────────

function PortalHome() {
  const { data, error, reload, setData } = useApi("portal:me", portalApi.me);
  usePageTitle(data ? data.address : "Your mailbox");
  const navigate = useNavigate();

  async function signOut() {
    try {
      await portalApi.logout();
    } finally {
      navigate("/account/login", { replace: true });
    }
  }

  return (
    <div className="min-h-dvh bg-background">
      <header className="sticky top-0 z-30 border-b border-border bg-background/80 backdrop-blur-md">
        <div className="mx-auto flex h-14 max-w-6xl items-center gap-3 px-4 sm:px-6 lg:px-8">
          <span className="grid h-8 w-8 place-items-center rounded-lg border border-border bg-card shadow-card">
            <YunoriMark className="h-[18px] w-[18px]" />
          </span>
          <span className="text-sm font-semibold tracking-tight">Yunori Mail</span>
          <span aria-hidden className="hidden h-4 w-px bg-border sm:block" />
          <span className="hidden text-sm text-muted-foreground sm:block">Your mailbox</span>
          <div className="ml-auto flex items-center gap-1.5">
            <ThemeToggle />
            {data && (
              <Button variant="ghost" size="sm" onClick={() => void signOut()}>
                <LogOut className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">Sign out</span>
                <span className="sr-only sm:hidden">Sign out</span>
              </Button>
            )}
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-6xl px-4 pb-20 pt-6 sm:px-6 sm:pt-10 lg:px-8">
        {error && !data ? (
          <LoadError error={error} onRetry={() => void reload()} what="Your mailbox" />
        ) : !data ? (
          <div aria-busy="true" className="lg:pl-[15rem]">
            <Skeleton className="h-36 rounded-2xl" />
            <Skeleton className="mt-6 h-56 rounded-xl" />
            <Skeleton className="mt-6 h-56 rounded-xl" />
          </div>
        ) : (
          <Home me={data} onChange={(me) => setData(me)} />
        )}
      </main>
    </div>
  );
}

const SECTIONS = [
  { id: "sec-connect-your-devices", label: "Devices", icon: Smartphone },
  { id: "sec-move-your-mail-here", label: "Move your mail here", icon: ArrowDownToLine },
  { id: "sec-auto-reply-forwarding", label: "Auto-reply & forwarding", icon: Reply },
  { id: "sec-mail-settings", label: "Mail settings", icon: Settings2 },
  { id: "sec-password", label: "Password", icon: Lock },
  { id: "sec-app-passwords", label: "App passwords", icon: KeyRound },
] as const;

function Home({ me, onChange }: { me: PortalMe; onChange: (me: PortalMe) => void }) {
  const [params, setParams] = useSearchParams();
  // Arrived from an invite: the password is set, devices are next.
  const [welcome] = useState(() => params.get("welcome") === "1");
  const connectRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!welcome) return;
    setParams({}, { replace: true });
    connectRef.current?.scrollIntoView({ block: "start" });
  }, [welcome, setParams]);
  const p = pct(me.usedBytes, me.quotaBytes);
  const used = fmtBytes(me.usedBytes ?? 0);
  const storageText = me.quotaBytes ? `${used} of ${fmtBytes(me.quotaBytes)} used` : `${used} used · no limit`;

  return (
    <div className="lg:grid lg:grid-cols-[13rem_minmax(0,1fr)] lg:gap-10">
      <nav aria-label="On this page" className="hidden lg:block">
        <ul className="sticky top-24 space-y-0.5">
          {SECTIONS.map((s) => (
            <li key={s.id}>
              <a
                href={`#${s.id}`}
                className="flex h-9 items-center gap-2.5 rounded-lg px-2.5 text-[13.5px] text-foreground/70 transition-colors hover:bg-accent/70 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
              >
                <s.icon aria-hidden className="h-4 w-4 text-icon" strokeWidth={1.75} />
                {s.label}
              </a>
            </li>
          ))}
        </ul>
      </nav>

      <div className="min-w-0 max-w-3xl">
        <section aria-labelledby="me-title" className="relative overflow-hidden rounded-2xl border border-border bg-card p-5 shadow-card sm:p-7">
          <div className="relative flex flex-col gap-5 sm:flex-row sm:items-center sm:justify-between">
            <div className="flex min-w-0 items-center gap-4">
              <Avatar name={me.displayName ?? me.address} className="h-14 w-14 text-lg" />
              <div className="min-w-0">
                <p className="text-xs font-medium text-muted-foreground">Your mailbox</p>
                <h1 id="me-title" className="mt-0.5 font-serif text-[clamp(1.6rem,5vw,2.25rem)] leading-[1.1] tracking-[-0.02em] [overflow-wrap:anywhere]">
                  {me.displayName ?? me.address}
                </h1>
                <div className="mt-1 flex items-center gap-1">
                  <span className="break-all font-mono text-[13px] text-muted-foreground">{me.address}</span>
                  <CopyButton value={me.address} label="your address" />
                </div>
              </div>
            </div>
            <div className="flex items-center gap-3 rounded-xl border border-border bg-muted/40 p-3 sm:min-w-[13rem]">
              <RingMeter value={p} label="Mailbox storage" valueText={storageText} size={56} stroke={6} />
              <div className="min-w-0">
                <p className="text-[13px] font-medium text-foreground">Storage</p>
                <p className="text-xs tabular-nums text-muted-foreground">{storageText}</p>
                {p !== null && p >= 90 && <p className="text-xs font-medium text-warning-ink">Nearly full — new mail is refused once it's full</p>}
              </div>
            </div>
          </div>
        </section>

        <div ref={connectRef} className="scroll-mt-20">
          {welcome && (
            <Callout role="status" tone="ok" className="mt-6">
              Your password is set and your mailbox is ready. Now connect the devices you'll read it on.
            </Callout>
          )}
          <Section
            icon={Smartphone}
            title="Connect your devices"
            blurb="Each phone, computer or app can get its own app password, so you can sign one out without changing your password."
          >
            <ConnectDevices me={me} onChange={onChange} />
          </Section>
        </div>

        <Section
          icon={ArrowDownToLine}
          title="Move your mail here"
          blurb="Bring your mail over from Gmail, Microsoft 365 or another provider — folders, read and starred marks, and dates included."
        >
          <MoveMail address={me.address} />
        </Section>

        <Section
          icon={Reply}
          id="sec-auto-reply-forwarding"
          title="Auto-reply & forwarding"
          blurb="Answer mail while you're away, or send it on to another address. Both run on the server, so they work with every app closed."
        >
          <MailRulesCard address={me.address} />
        </Section>

        <Section icon={Settings2} title="Mail settings" blurb="Most apps find these on their own. If yours asks, use these — your username is your full address.">
          <dl className="divide-y divide-border/70 rounded-xl border border-border bg-card shadow-card">
            <SettingRow label="Username" value={me.address} />
            <SettingRow label="Incoming (IMAP)" value={me.settings.imap.host} extra={`Port ${me.settings.imap.port} · ${me.settings.imap.security}`} />
            <SettingRow label="Outgoing (SMTP)" value={me.settings.smtp.host} extra={`Port ${me.settings.smtp.port} · ${me.settings.smtp.security}`} />
            <SettingRow label="Calendars & contacts" value={me.settings.dav.url} extra="CalDAV · CardDAV" />
          </dl>
        </Section>

        <Section icon={Lock} title="Password" blurb="Changing it signs you out of mail apps that use it. App passwords keep working.">
          <ChangePassword address={me.address} />
        </Section>

        <Section icon={KeyRound} title="App passwords" blurb="A separate password for each app or device. Revoke one without changing your main password.">
          <AppPasswords me={me} onChange={onChange} />
        </Section>
      </div>
    </div>
  );
}

function Section({ title, blurb, icon: Icon, id: givenId, children }: { title: string; blurb?: string; icon: LucideIcon; id?: string; children: ReactNode }) {
  const id = givenId ?? `sec-${title.toLowerCase().replace(/\W+/g, "-")}`;
  return (
    <section aria-labelledby={`${id}-h`} id={id} className="mt-12 scroll-mt-20">
      <div className="mb-4 flex items-start gap-3">
        <span aria-hidden className="mt-0.5 grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-border bg-card text-icon shadow-card">
          <Icon className="h-4 w-4" strokeWidth={1.75} />
        </span>
        <div className="min-w-0">
          <h2 id={`${id}-h`} className="text-base font-semibold tracking-[-0.01em]">
            {title}
          </h2>
          {blurb && <p className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">{blurb}</p>}
        </div>
      </div>
      {children}
    </section>
  );
}

function SettingRow({ label, value, extra }: { label: string; value: string; extra?: string }) {
  return (
    <div className="flex flex-col gap-1 px-4 py-3 sm:flex-row sm:items-center sm:gap-4 sm:px-5">
      <dt className="w-40 shrink-0 text-[13px] text-muted-foreground">{label}</dt>
      <dd className="flex min-w-0 flex-1 flex-wrap items-center gap-x-1 gap-y-0.5">
        <span className="min-w-0 break-all font-mono text-[13px] text-foreground">{value}</span>
        <CopyButton value={value} label={label} />
        {extra && <span className="w-full text-xs text-muted-foreground sm:ml-auto sm:w-auto sm:shrink-0 sm:pl-2">{extra}</span>}
      </dd>
    </div>
  );
}

function ChangePassword({ address }: { address: string }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const errs: Record<string, string> = {};
    if (!current) errs.current = "Enter your current password.";
    if (next.length < MIN_PASSWORD) errs.next = `Use at least ${MIN_PASSWORD} characters.`;
    else if (confirm !== next) errs.confirm = "The two new passwords don't match.";
    setErrors(errs);
    if (Object.keys(errs).length) return;
    setBusy(true);
    try {
      await portalApi.changePassword({ current, next });
      setCurrent("");
      setNext("");
      setConfirm("");
      toast.success("Password changed. Update it in any mail app that asks.");
    } catch (err) {
      if (err instanceof ApiRequestError && err.field) setErrors({ [err.field]: err.message });
      else setErrors({ form: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="space-y-4 rounded-xl border border-border bg-card p-5 shadow-card" noValidate>
      <input type="text" autoComplete="username" value={address} hidden readOnly aria-hidden tabIndex={-1} />
      <div className="space-y-1.5">
        <Label htmlFor="pw-current">
          Current password
        </Label>
        <PasswordInput
          id="pw-current"
          autoComplete="current-password"
          value={current}
          onChange={(e) => setCurrent(e.target.value)}
          aria-invalid={!!errors.current}
          aria-describedby={errors.current ? "pw-current-err" : undefined}
        />
        <FieldError id="pw-current-err">{errors.current}</FieldError>
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="pw-next">
            New password
          </Label>
          <PasswordInput
            id="pw-next"
            autoComplete="new-password"
            value={next}
            onChange={(e) => setNext(e.target.value)}
            aria-invalid={!!errors.next}
            aria-describedby={errors.next ? "pw-next-err" : "pw-next-strength"}
          />
          {errors.next ? <FieldError id="pw-next-err">{errors.next}</FieldError> : <StrengthHint password={next} id="pw-next-strength" />}
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="pw-confirm">
            Repeat new password
          </Label>
          <PasswordInput
            id="pw-confirm"
            autoComplete="new-password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            aria-invalid={!!errors.confirm}
            aria-describedby={errors.confirm ? "pw-confirm-err" : undefined}
          />
          <FieldError id="pw-confirm-err">{errors.confirm}</FieldError>
        </div>
      </div>
      {errors.form && <FormError>{errors.form}</FormError>}
      <div className="flex justify-end">
        <Button type="submit" size="sm" loading={busy}>
          {busy ? "Changing…" : "Change password"}
        </Button>
      </div>
    </form>
  );
}

function AppPasswords({ me, onChange }: { me: PortalMe; onChange: (me: PortalMe) => void }) {
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<{ description: string; secret: string } | null>(null);
  const [revoking, setRevoking] = useState<AppPassword | null>(null);

  async function create(e: React.FormEvent) {
    e.preventDefault();
    if (!description.trim()) {
      setError("Name it after the app or device, like “Phone” or “Thunderbird”.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const r = await portalApi.createAppPassword(description.trim());
      setCreated({ description: r.appPassword.description, secret: r.secret });
      onChange({ ...me, appPasswords: [...me.appPasswords, r.appPassword] });
      setDescription("");
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="rounded-xl border border-border bg-card shadow-card">
      {me.appPasswords.length > 0 ? (
        <ul className="divide-y divide-border/70">
          {me.appPasswords.map((a) => (
            <li key={a.id} className="flex items-center justify-between gap-3 px-4 py-3 sm:px-5">
              <div className="flex min-w-0 items-center gap-3">
                <span aria-hidden className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-border bg-muted/50 text-icon">
                  <KeyRound className="h-4 w-4" strokeWidth={1.75} />
                </span>
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium text-foreground">{a.description}</p>
                  <p className="text-xs text-muted-foreground">Created {fmtDate(a.createdAt)}</p>
                </div>
              </div>
              <Button variant="ghost" size="sm" onClick={() => setRevoking(a)} className="text-muted-foreground hover:text-danger-ink" aria-label={`Revoke ${a.description}`}>
                <Trash2 className="h-3.5 w-3.5" />
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="px-5 py-4 text-[13px] text-muted-foreground">No app passwords yet.</p>
      )}

      <div className="rounded-b-xl border-t border-border/70 bg-muted/25 p-4 sm:p-5">
        {created && (
          <div className="mb-4">
            <SecretReveal
              label={`App password for ${created.description}`}
              value={created.secret}
              note="Shown once. Paste it into the app's password field (spaces are optional). You can revoke it here at any time."
            >
              <div className="mt-3 flex justify-end">
                <Button size="sm" variant="outline" onClick={() => setCreated(null)}>
                  Done
                </Button>
              </div>
            </SecretReveal>
          </div>
        )}
        <form onSubmit={create} className="flex flex-col gap-2 sm:flex-row sm:items-start" noValidate>
          <div className="flex-1 space-y-1.5">
            <label htmlFor="ap-desc" className="sr-only">
              App or device name
            </label>
            <Input
              id="ap-desc"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="App or device, e.g. “Work laptop”"
              maxLength={80}
              aria-invalid={!!error}
              aria-describedby={error ? "ap-desc-err" : undefined}
            />
            <FieldError id="ap-desc-err">{error}</FieldError>
          </div>
          <Button type="submit" variant="outline" loading={busy} className="shrink-0">
            {!busy && <Plus className="h-4 w-4" />}
            {busy ? "Creating…" : "Create app password"}
          </Button>
        </form>
      </div>

      <ConfirmAction
        open={!!revoking}
        onOpenChange={(o) => !o && setRevoking(null)}
        title={`Revoke “${revoking?.description ?? ""}”?`}
        description="The app using it will stop syncing until you sign it in again with a new password."
        confirmLabel="Revoke"
        destructive
        onConfirm={async () => {
          const a = revoking!;
          await portalApi.revokeAppPassword(a.id);
          onChange({ ...me, appPasswords: me.appPasswords.filter((x) => x.id !== a.id) });
          toast.success(`“${a.description}” was revoked.`);
        }}
      />
    </div>
  );
}
