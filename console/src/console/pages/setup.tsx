import { useState } from "react";
import { Navigate, useNavigate } from "react-router-dom";
import { KeyRound, ScanLine } from "lucide-react";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { SetupResponse } from "@/api/types";
import { AuthFrame } from "@/components/auth-frame";
import { CopyButton } from "@/components/copy-button";
import { PasswordInput, StrengthHint } from "@/components/password-input";
import { QrCode } from "@/components/qr-code";
import { MIN_PASSWORD } from "@/lib/password";
import { Button } from "@/ui/button";
import { Callout, FormError } from "@/ui/feedback";
import { FieldError, FieldHint, Input, Label } from "@/ui/form";
import { usePageTitle } from "@/ui/page";
import { Step, Steps } from "@/ui/steps";
import { useSession } from "../session";

type FieldErrors = Partial<Record<"token" | "email" | "password" | "code", string>>;

let linkToken: string | null = null;
/**
 * The installer prints a one-time link, `/console/setup#token=…`. The token
 * rides in the fragment, which browsers never send to the server (no access
 * log, no Referer). Read it once and take it out of the address bar and the
 * history entry straight away.
 */
function tokenFromLink(): string {
  if (linkToken === null) {
    const m = /(?:^#|&)token=([^&]+)/.exec(window.location.hash);
    linkToken = m ? decodeURIComponent(m[1]).trim() : "";
    if (m) window.history.replaceState(window.history.state, "", window.location.pathname + window.location.search);
  }
  return linkToken;
}

/**
 * First run: the installer printed a one-time setup token. Step 1 creates the
 * owner; step 2 pairs an authenticator (mandatory) and signs in.
 */
export default function SetupPage() {
  usePageTitle("Set up");
  const { session, refresh } = useSession();
  const navigate = useNavigate();
  const [totp, setTotp] = useState<SetupResponse | null>(null);
  const [email, setEmail] = useState("");
  const [linkTokenValue] = useState(tokenFromLink);

  // Owner just created (or already there): the onboarding wizard until setup is complete.
  if (!session.setupRequired)
    return <Navigate to={!session.authenticated ? "/console/login" : session.server.installComplete ? "/console" : "/console/onboarding"} replace />;

  return totp ? (
    <TotpStep
      totp={totp}
      email={email}
      onDone={async () => {
        const s = await refresh();
        // Setup not finished yet: the browser onboarding takes over from the terminal.
        navigate(s.server.installComplete ? "/console" : "/console/onboarding", { replace: true });
      }}
    />
  ) : (
    <OwnerStep
      hostname={session.server.consoleHost ?? session.server.hostname}
      initialToken={linkTokenValue}
      onDone={(r, e) => {
        setEmail(e);
        setTotp(r);
      }}
    />
  );
}

function OwnerStep({
  hostname,
  initialToken,
  onDone,
}: {
  hostname: string;
  initialToken: string;
  onDone: (r: SetupResponse, email: string) => void;
}) {
  const [token, setToken] = useState(initialToken);
  // Arrived through the installer's link: the token is already here.
  const [showToken, setShowToken] = useState(!initialToken);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<FieldErrors>({});
  const [formError, setFormError] = useState<string | null>(null);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setErrors({});
    setFormError(null);
    const local: FieldErrors = {};
    if (!token.trim()) {
      local.token = "Paste the setup token from the installer.";
      setShowToken(true);
    }
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim())) local.email = "Enter a valid email address.";
    if (password.length < MIN_PASSWORD) local.password = `Use at least ${MIN_PASSWORD} characters.`;
    if (Object.keys(local).length) {
      setErrors(local);
      return;
    }
    setBusy(true);
    try {
      const r = await api.setup({ token: token.trim(), email: email.trim(), password });
      onDone(r, email.trim());
    } catch (err) {
      if (err instanceof ApiRequestError && err.field && ["token", "email", "password"].includes(err.field)) {
        if (err.field === "token") setShowToken(true);
        setErrors({ [err.field]: err.message });
      } else {
        setFormError(errorMessage(err));
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthFrame
      eyebrow="Step 1 of 2 · Owner account"
      title="Set up Yunori Mail"
      subtitle={
        <>
          Create the first owner for <span className="font-mono text-[13px] text-foreground/85">{hostname}</span>.
        </>
      }
      aside={<SetupJourney stage={1} />}
      footnote={
        <>
          The link works once, for this server only. Lost it? Run <code className="font-mono text-foreground/80">sudo yunori-mail setup-token</code> on the
          server for a new one.
        </>
      }
    >
      <form onSubmit={onSubmit} className="space-y-5" noValidate>
        {showToken ? (
          <div className="space-y-1.5">
            <Label htmlFor="token">Setup token</Label>
            <Input
              id="token"
              autoComplete="off"
              spellCheck={false}
              autoFocus
              value={token}
              onChange={(e) => setToken(e.target.value)}
              className="font-mono"
              aria-invalid={!!errors.token}
              aria-describedby={errors.token ? "token-err" : "token-hint"}
            />
            {errors.token ? (
              <FieldError id="token-err">{errors.token}</FieldError>
            ) : (
              <FieldHint id="token-hint">
                Printed by <code className="font-mono">yunori-mail install</code> (or <code className="font-mono">yunori-mail setup-token</code>).
              </FieldHint>
            )}
          </div>
        ) : (
          <Callout
            tone="ok"
            className="py-3"
            action={
              <Button type="button" variant="ghost" size="sm" onClick={() => setShowToken(true)} className="h-7 px-2 text-xs text-muted-foreground">
                Use another token
              </Button>
            }
          >
            Setup link recognised. Choose how you'll sign in to this console.
          </Callout>
        )}
        <div className="space-y-1.5">
          <Label htmlFor="email">Your email</Label>
          <Input
            id="email"
            type="email"
            autoComplete="username"
            autoFocus={!showToken}
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            aria-invalid={!!errors.email}
            aria-describedby={errors.email ? "email-err" : "email-hint"}
          />
          {errors.email ? (
            <FieldError id="email-err">{errors.email}</FieldError>
          ) : (
            <FieldHint id="email-hint">Used to sign in to this console. It doesn't need to be on this server.</FieldHint>
          )}
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="password">Password</Label>
          <PasswordInput
            id="password"
            autoComplete="new-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            aria-invalid={!!errors.password}
            aria-describedby={errors.password ? "password-err" : "password-strength"}
          />
          {errors.password ? <FieldError id="password-err">{errors.password}</FieldError> : <StrengthHint password={password} id="password-strength" />}
        </div>
        <FormError>{formError}</FormError>
        <Button type="submit" variant="torii" className="h-10 w-full" loading={busy}>
          {busy ? "Creating…" : "Continue"}
        </Button>
      </form>
    </AuthFrame>
  );
}

function TotpStep({ totp, email, onDone }: { totp: SetupResponse; email: string; onDone: () => Promise<void> }) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const grouped = totp.totpSecret.match(/.{1,4}/g)?.join(" ") ?? totp.totpSecret;

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (!/^\d{6}$/.test(code)) {
      setError("Enter the 6-digit code your app shows now.");
      return;
    }
    setBusy(true);
    try {
      await api.setupTotp({ code });
      await onDone();
    } catch (err) {
      setCode("");
      setError(errorMessage(err));
      setBusy(false);
    }
  }

  return (
    <AuthFrame
      eyebrow="Step 2 of 2 · Two-step sign-in"
      title="Pair your authenticator"
      subtitle="Every console sign-in needs a code. Scan this with 1Password, Bitwarden, Aegis or any TOTP app."
      width="max-w-md"
      aside={<SetupJourney stage={2} />}
      footnote={<>Signed in as {email} once confirmed. The secret is never shown again.</>}
    >
      <div className="overflow-hidden rounded-xl border border-border bg-card shadow-card">
        <div className="flex flex-col items-center gap-4 px-6 pb-5 pt-5">
          <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <ScanLine aria-hidden className="h-3.5 w-3.5" />
            Scan with your authenticator app
          </p>
          <div className="rounded-xl border border-border/70 bg-[#faf7f2] p-2.5 shadow-card">
            <QrCode value={totp.otpauthUrl} label={`QR code for two-step sign-in as ${email}`} />
          </div>
        </div>
        <div className="border-t border-border/70 bg-muted/30 px-4 py-3.5">
          <p className="flex items-center justify-center gap-1.5 text-center text-xs text-muted-foreground">
            <KeyRound aria-hidden className="h-3.5 w-3.5" />
            Can't scan? Enter this key instead:
          </p>
          <div className="mt-1.5 flex items-center justify-center gap-1">
            <code className="min-w-0 select-all break-all text-center font-mono text-[13px] tracking-wider text-foreground" aria-label="Secret key">
              {grouped}
            </code>
            <CopyButton value={totp.totpSecret} label="secret key" />
          </div>
        </div>
      </div>

      <form onSubmit={onSubmit} className="mt-6 space-y-4" noValidate>
        <div className="space-y-1.5">
          <Label htmlFor="code">6-digit code</Label>
          <Input
            id="code"
            inputMode="numeric"
            autoComplete="one-time-code"
            maxLength={6}
            autoFocus
            placeholder="123 456"
            value={code}
            onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
            className="h-11 text-center font-mono text-lg tracking-[0.35em] placeholder:tracking-[0.2em]"
            aria-invalid={!!error}
            aria-describedby={error ? "code-err" : undefined}
          />
          <FieldError id="code-err">{error}</FieldError>
        </div>
        <Button type="submit" variant="torii" className="h-10 w-full" loading={busy}>
          {busy ? "Confirming…" : "Confirm and sign in"}
        </Button>
      </form>
    </AuthFrame>
  );
}

/** The brand panel during first run: where this sits in the whole setup. */
function SetupJourney({ stage }: { stage: 1 | 2 }) {
  return (
    <>
      <p className="font-serif text-[32px] leading-[1.14] tracking-[-0.02em]">Your own mail server, a few calm steps away.</p>
      <p className="mt-4 max-w-sm text-sm leading-relaxed text-muted-foreground">
        Secure this console first. Then the browser walks you through your domain, DNS and certificate, checking each one for real.
      </p>
      <Steps label="Setup stages" className="mt-9">
        <Step index={1} state={stage === 1 ? "current" : "done"} title="Owner account" meta={stage === 1 ? "Now" : undefined}>
          Your sign-in for this console.
        </Step>
        <Step index={2} state={stage === 2 ? "current" : "pending"} title="Two-step sign-in" meta={stage === 2 ? "Now" : undefined}>
          A code from your authenticator app.
        </Step>
        <Step index={3} state="pending" title="Domain, DNS and certificate">
          Checked against public resolvers and Let's Encrypt.
        </Step>
        <Step index={4} state="pending" title="Your first mailbox" last>
          Then connect any mail app.
        </Step>
      </Steps>
    </>
  );
}
