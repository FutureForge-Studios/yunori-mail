import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { Navigate, Outlet, useLocation } from "react-router-dom";
import { RotateCw } from "lucide-react";
import { ToriLogo } from "@/design/vendor/logo";
import { Button } from "@/design/vendor/ui/button";
import { api, errorMessage } from "@/api/client";
import type { SessionResponse } from "@/api/types";

interface SessionCtx {
  session: SessionResponse;
  setSession: (s: SessionResponse) => void;
  refresh: () => Promise<SessionResponse>;
}

const Ctx = createContext<SessionCtx | null>(null);

export function useSession(): SessionCtx {
  const v = useContext(Ctx);
  if (!v) throw new Error("useSession outside ConsoleRoot");
  return v;
}

/** Calm full-screen splash (same as the client admin's loading state). */
export function Splash({ label = "Console" }: { label?: string }) {
  return (
    <div className="grid h-dvh place-items-center bg-background" aria-busy="true" aria-live="polite">
      <div className="flex flex-col items-center gap-3">
        <ToriLogo className="h-7 w-7 animate-pulse opacity-70 motion-reduce:animate-none" />
        <span className="text-[11px] uppercase tracking-[0.18em] text-muted-foreground">{label}</span>
      </div>
    </div>
  );
}

/** Loads GET /session once for everything under /console. */
export function ConsoleRoot() {
  const [session, setSession] = useState<SessionResponse | null>(null);
  const [error, setError] = useState<unknown>(null);

  const refresh = useCallback(async () => {
    const s = await api.session();
    setSession(s);
    setError(null);
    return s;
  }, []);

  useEffect(() => {
    refresh().catch(setError);
  }, [refresh]);

  if (error && !session) {
    return (
      <div className="grid h-dvh place-items-center bg-background px-6">
        <div className="max-w-sm text-center">
          <ToriLogo className="mx-auto h-8 w-8 opacity-70" />
          <h1 className="mt-5 font-serif text-xl tracking-tight">The console can't reach the server</h1>
          <p className="mt-2 text-sm text-muted-foreground">
            {errorMessage(error, "yunori-mail didn't answer.")} If you just restarted it, give it a few seconds.
          </p>
          <Button variant="outline" size="sm" className="mt-6" onClick={() => refresh().catch(setError)}>
            <RotateCw className="h-3.5 w-3.5" />
            Try again
          </Button>
        </div>
      </div>
    );
  }
  if (!session) return <Splash />;

  return (
    <Ctx.Provider value={{ session, setSession, refresh }}>
      <Outlet />
    </Ctx.Provider>
  );
}

/** Gate for the signed-in console. Setup beats login beats content. */
export function RequireAdmin({ children }: { children: ReactNode }) {
  const { session } = useSession();
  const { pathname, search } = useLocation();
  if (session.setupRequired) return <Navigate to="/console/setup" replace />;
  if (!session.authenticated) {
    const next = `${pathname}${search}`;
    return <Navigate to={next === "/console" ? "/console/login" : `/console/login?next=${encodeURIComponent(next)}`} replace />;
  }
  return <>{children}</>;
}

/** Only same-app console paths are honoured as post-login destinations. */
export function safeConsoleNext(raw: string | null): string {
  if (!raw || !raw.startsWith("/console") || raw.startsWith("//") || raw.includes("\\")) return "/console";
  if (raw.startsWith("/console/login") || raw.startsWith("/console/setup")) return "/console";
  return raw;
}
