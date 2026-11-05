import { useState } from "react";
import { Navigate, useNavigate, useSearchParams } from "react-router-dom";
import { Button } from "@/design/vendor/ui/button";
import { Input } from "@/design/vendor/ui/input";
import { Label } from "@/design/vendor/ui/label";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import { AuthFrame, FormError } from "@/components/auth-frame";
import { PasswordInput } from "@/components/password-input";
import { usePageTitle } from "@/components/kit";
import { safeConsoleNext, useSession } from "../session";

/**
 * Admin sign-in: email, password and the 6-digit code together. The error
 * never says which of the three was wrong (non-enumerating).
 */
export function LoginPage() {
  usePageTitle("Sign in");
  const { session, setSession } = useSession();
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const next = safeConsoleNext(params.get("next"));
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (session.setupRequired) return <Navigate to="/console/setup" replace />;
  if (session.authenticated) return <Navigate to={next} replace />;

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (!email.trim() || !password) {
      setError("Enter your email and password.");
      return;
    }
    if (!/^\d{6}$/.test(code)) {
      setError("Enter the 6-digit code from your authenticator app.");
      return;
    }
    setBusy(true);
    try {
      const s = await api.login({ email: email.trim(), password, totp: code });
      setSession(s);
      navigate(next, { replace: true });
    } catch (err) {
      setCode("");
      if (err instanceof ApiRequestError && (err.status === 401 || err.code === "invalid_credentials")) {
        setError("That combination didn't work. Check your email, password and the current code from your authenticator, then try again.");
      } else if (err instanceof ApiRequestError && err.status === 429) {
        setError("Too many attempts. Wait a minute before trying again.");
      } else {
        setError(errorMessage(err));
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <AuthFrame
      title="Sign in to Yunori Mail"
      subtitle={<span className="font-mono text-[13px]">{session.server.hostname}</span>}
      footnote={
        <>
          Lost your authenticator or password? Recovery happens on the server itself, with the recovery kit —
          console sign-in is never reset by email.
        </>
      }
    >
      <form onSubmit={onSubmit} className="space-y-4" noValidate>
        <div className="space-y-1.5">
          <Label htmlFor="email">Email</Label>
          <Input
            id="email"
            type="email"
            autoComplete="username"
            required
            autoFocus
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="password">Password</Label>
          <PasswordInput
            id="password"
            autoComplete="current-password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="code">Authenticator code</Label>
          <Input
            id="code"
            inputMode="numeric"
            autoComplete="one-time-code"
            pattern="\d{6}"
            maxLength={6}
            required
            placeholder="123 456"
            value={code}
            onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
            className="font-mono tracking-[0.3em]"
            aria-describedby="code-hint"
          />
          <p id="code-hint" className="text-[11px] text-muted-foreground">
            The 6-digit code from the app you paired during setup.
          </p>
        </div>
        <FormError>{error}</FormError>
        <Button type="submit" variant="torii" className="w-full" disabled={busy}>
          {busy ? "Signing in…" : "Sign in"}
        </Button>
      </form>
    </AuthFrame>
  );
}
