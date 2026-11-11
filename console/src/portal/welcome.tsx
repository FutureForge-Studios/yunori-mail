import { useEffect, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { Button } from "@/ui/button";
import { Label } from "@/ui/form";
import { ApiRequestError, errorMessage, portalApi } from "@/api/client";
import type { InviteInfo } from "@/api/types";
import { AuthFrame, FormError } from "@/components/auth-frame";
import { FieldError, usePageTitle } from "@/components/kit";
import { PasswordInput, StrengthHint } from "@/components/password-input";
import { fmtDate } from "@/lib/format";
import { MIN_PASSWORD } from "@/lib/password";

/**
 * Reads `#invite=…` once and takes it out of the address bar, so the link is
 * not left in history or on screen (same as the console's setup link).
 */
let lastToken = "";
function tokenFromLink(): string {
  const m = /(?:^#|&)invite=([^&]+)/.exec(window.location.hash);
  if (m) {
    window.history.replaceState(window.history.state, "", window.location.pathname + window.location.search);
    lastToken = decodeURIComponent(m[1]);
  }
  // Remembered so a second call (React StrictMode runs initialisers twice) still sees it.
  return lastToken;
}

/** /account/welcome#invite=… — an invited person chooses their own password, then sets up their devices. */
export function PortalWelcome() {
  const navigate = useNavigate();
  const [token] = useState(tokenFromLink);
  const [info, setInfo] = useState<InviteInfo | null>(null);
  const [problem, setProblem] = useState<string | null>(
    token ? null : "This page opens from the invite link you were sent. Open that link again, or ask for a new one.",
  );
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});
  usePageTitle(info ? `Welcome, ${info.displayName ?? info.address}` : "Your invitation");

  useEffect(() => {
    if (!token) return;
    let cancelled = false;
    portalApi
      .inviteInfo(token)
      .then((i) => !cancelled && setInfo(i))
      .catch((err) => !cancelled && setProblem(errorMessage(err)));
    return () => {
      cancelled = true;
    };
  }, [token]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const errs: Record<string, string> = {};
    if (password.length < MIN_PASSWORD) errs.password = `Use at least ${MIN_PASSWORD} characters.`;
    else if (confirm !== password) errs.confirm = "The two passwords don't match.";
    setErrors(errs);
    if (Object.keys(errs).length) return;
    setBusy(true);
    try {
      await portalApi.acceptInvite({ token, password });
      navigate("/account?welcome=1", { replace: true });
    } catch (err) {
      if (err instanceof ApiRequestError && err.code === "invite_invalid") setProblem(err.message);
      else if (err instanceof ApiRequestError && err.field) setErrors({ [err.field]: err.message });
      else setErrors({ form: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  if (problem) {
    return (
      <AuthFrame eyebrow="Yunori Mail" title="This link doesn't work" subtitle={problem}>
        <p className="text-center text-sm">
          Already chose a password?{" "}
          <Link to="/account/login" className="underline underline-offset-4">
            Sign in
          </Link>
        </p>
      </AuthFrame>
    );
  }

  if (!info) return <div className="min-h-dvh bg-background" aria-busy="true" />;

  return (
    <AuthFrame
      eyebrow="Yunori Mail"
      title={info.displayName ? `Welcome, ${info.displayName.split(" ")[0]}` : "Welcome"}
      subtitle={
        <>
          Choose a password for <span className="break-all font-mono text-foreground/90">{info.address}</span>.
        </>
      }
      footnote={`Only you will know it — not even the person who runs this server. This link works once, until ${fmtDate(info.expiresAt)}.`}
    >
      <form onSubmit={submit} className="space-y-4" noValidate>
        <input type="text" autoComplete="username" value={info.address} hidden readOnly aria-hidden tabIndex={-1} />
        <div className="space-y-1.5">
          <Label htmlFor="w-password">New password</Label>
          <PasswordInput
            id="w-password"
            autoComplete="new-password"
            autoFocus
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            aria-invalid={!!errors.password}
            aria-describedby={errors.password ? "w-password-err" : "w-password-strength"}
          />
          {errors.password ? <FieldError id="w-password-err">{errors.password}</FieldError> : <StrengthHint password={password} id="w-password-strength" />}
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="w-confirm">Repeat it</Label>
          <PasswordInput
            id="w-confirm"
            autoComplete="new-password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            aria-invalid={!!errors.confirm}
            aria-describedby={errors.confirm ? "w-confirm-err" : undefined}
          />
          <FieldError id="w-confirm-err">{errors.confirm}</FieldError>
        </div>
        <FormError>{errors.form}</FormError>
        <Button type="submit" variant="torii" size="lg" className="w-full" loading={busy}>
          {busy ? "Setting it up…" : "Set password and continue"}
        </Button>
      </form>
    </AuthFrame>
  );
}
