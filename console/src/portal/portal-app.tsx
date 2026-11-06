import { useEffect, useRef, useState, type ReactNode } from "react";
import { Route, Routes, useNavigate, useSearchParams } from "react-router-dom";
import { CheckCircle2, LogOut, Plus, Trash2 } from "lucide-react";
import { YunoriMark } from "@/components/yunori-logo";
import { Button } from "@/design/vendor/ui/button";
import { Input } from "@/design/vendor/ui/input";
import { Label } from "@/design/vendor/ui/label";
import { Skeleton } from "@/design/vendor/admin/ui";
import { ApiRequestError, errorMessage, portalApi } from "@/api/client";
import type { AppPassword, PortalMe } from "@/api/types";
import { AuthFrame, FormError } from "@/components/auth-frame";
import { ConfirmAction } from "@/components/confirm";
import { CopyButton } from "@/components/copy-button";
import { FieldError, LoadError, usePageTitle } from "@/components/kit";
import { Meter } from "@/components/meter";
import { PasswordInput, StrengthHint } from "@/components/password-input";
import { SecretReveal } from "@/components/secret-reveal";
import { ThemeToggle } from "@/components/theme-toggle";
import { fmtBytes, fmtDate, pct } from "@/lib/format";
import { MIN_PASSWORD } from "@/lib/password";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";
import { NotFound } from "@/pages/not-found";
import { ConnectDevices } from "./devices";
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
        <Button type="submit" variant="torii" className="w-full" disabled={busy}>
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
      <header className="border-b border-border/60">
        <div className="mx-auto flex h-14 max-w-3xl items-center gap-2.5 px-5">
          <YunoriMark className="h-5 w-5" />
          <span className="text-sm font-medium tracking-tight">Yunori Mail</span>
          <div className="ml-auto flex items-center gap-1">
            <ThemeToggle />
            {data && (
              <Button variant="ghost" size="sm" onClick={() => void signOut()} className="text-muted-foreground">
                <LogOut className="h-3.5 w-3.5" />
                Sign out
              </Button>
            )}
          </div>
        </div>
      </header>

      <main className="mx-auto max-w-3xl px-5 pb-16 pt-10 sm:pt-14">
        {error && !data ? (
          <LoadError error={error} onRetry={() => void reload()} what="Your mailbox" />
        ) : !data ? (
          <div aria-busy="true">
            <Skeleton className="h-3 w-24" />
            <Skeleton className="mt-3 h-10 w-72" />
            <Skeleton className="mt-10 h-40" />
            <Skeleton className="mt-4 h-56" />
          </div>
        ) : (
          <Home me={data} onChange={(me) => setData(me)} />
        )}
      </main>
    </div>
  );
}

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
    <>
      <section aria-labelledby="me-title">
        <p className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground">Your mailbox</p>
        <h1 id="me-title" className="mt-2 font-serif text-[clamp(2rem,6vw,2.75rem)] leading-[1.05] tracking-tight">
          {me.displayName ?? me.address}
        </h1>
        <div className="mt-2 flex items-center gap-1">
          <span className="break-all font-mono text-sm text-muted-foreground">{me.address}</span>
          <CopyButton value={me.address} label="your address" />
        </div>

      </section>

      <div ref={connectRef} className="scroll-mt-6">
        {welcome && (
          <p role="status" className="mt-8 flex items-start gap-2 rounded-lg border border-emerald-600/30 bg-emerald-500/5 px-4 py-3 text-sm leading-relaxed">
            <CheckCircle2 aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-emerald-700 dark:text-emerald-400" />
            <span>Your password is set and your mailbox is ready. Now connect the devices you'll read it on.</span>
          </p>
        )}
        <Section title="Connect your devices" blurb="Each phone, computer or app can get its own app password, so you can sign one out without changing your password.">
          <ConnectDevices me={me} onChange={onChange} />
        </Section>
      </div>

      <Section title="Storage">
        <Meter value={p} label="Mailbox storage" valueText={storageText} />
        <p className="mt-2 text-xs tabular-nums text-foreground/85">
          {storageText}
          {p !== null && p >= 90 && <span className="text-amber-700 dark:text-amber-400"> · nearly full — new mail is refused once it's full</span>}
        </p>
      </Section>

      <Section title="Mail settings" blurb="Most apps find these on their own. If yours asks, use these — your username is your full address.">
        <dl className="divide-y divide-border/50 rounded-lg border border-border/60">
          <SettingRow label="Username" value={me.address} />
          <SettingRow label="Incoming (IMAP)" value={me.settings.imap.host} extra={`Port ${me.settings.imap.port} · ${me.settings.imap.security}`} />
          <SettingRow label="Outgoing (SMTP)" value={me.settings.smtp.host} extra={`Port ${me.settings.smtp.port} · ${me.settings.smtp.security}`} />
          <SettingRow label="Calendars & contacts" value={me.settings.dav.url} extra="CalDAV · CardDAV" />
        </dl>
      </Section>

      <Section title="Password" blurb="Changing it signs you out of mail apps that use it. App passwords keep working.">
        <ChangePassword address={me.address} />
      </Section>

      <Section title="App passwords" blurb="A separate password for each app or device. Revoke one without changing your main password.">
        <AppPasswords me={me} onChange={onChange} />
      </Section>
    </>
  );
}

function Section({ title, blurb, children }: { title: string; blurb?: string; children: ReactNode }) {
  const id = `sec-${title.toLowerCase().replace(/\W+/g, "-")}`;
  return (
    <section aria-labelledby={id} className="mt-12">
      <h2 id={id} className="text-base font-medium tracking-tight">
        {title}
      </h2>
      {blurb && <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{blurb}</p>}
      <div className="mt-4">{children}</div>
    </section>
  );
}

function SettingRow({ label, value, extra }: { label: string; value: string; extra?: string }) {
  return (
    <div className="flex flex-col gap-1 px-4 py-3 sm:flex-row sm:items-center sm:gap-4">
      <dt className="w-36 shrink-0 text-xs text-muted-foreground">{label}</dt>
      <dd className="flex min-w-0 flex-1 items-center gap-1">
        <span className="min-w-0 break-all font-mono text-[13px] text-foreground/90">{value}</span>
        <CopyButton value={value} label={label} />
        {extra && <span className="ml-auto shrink-0 pl-2 text-xs text-muted-foreground">{extra}</span>}
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
    <form onSubmit={submit} className="space-y-4 rounded-xl border border-border/60 bg-card/40 p-5" noValidate>
      <input type="text" autoComplete="username" value={address} hidden readOnly aria-hidden tabIndex={-1} />
      <div className="space-y-1.5">
        <Label htmlFor="pw-current" className="normal-case tracking-normal">
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
          <Label htmlFor="pw-next" className="normal-case tracking-normal">
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
          <Label htmlFor="pw-confirm" className="normal-case tracking-normal">
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
        <Button type="submit" size="sm" disabled={busy}>
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
    <div className="rounded-xl border border-border/60 bg-card/40">
      {me.appPasswords.length > 0 ? (
        <ul className="divide-y divide-border/50">
          {me.appPasswords.map((a) => (
            <li key={a.id} className="flex items-center justify-between gap-3 px-5 py-3">
              <div className="min-w-0">
                <p className="truncate text-sm text-foreground/90">{a.description}</p>
                <p className="text-[11px] text-muted-foreground">Created {fmtDate(a.createdAt)}</p>
              </div>
              <Button variant="ghost" size="sm" onClick={() => setRevoking(a)} className="text-muted-foreground hover:text-destructive" aria-label={`Revoke ${a.description}`}>
                <Trash2 className="h-3.5 w-3.5" />
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="px-5 py-4 text-xs text-muted-foreground">No app passwords yet.</p>
      )}

      <div className="border-t border-border/50 p-5">
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
          <Button type="submit" variant="outline" disabled={busy} className="shrink-0">
            <Plus className="h-4 w-4" />
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
