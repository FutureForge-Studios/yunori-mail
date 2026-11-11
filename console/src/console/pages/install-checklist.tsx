import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { ArrowRight, Hourglass, Loader2 } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Card } from "@/design/vendor/admin/ui";
import { api } from "@/api/client";
import type { FirstMailboxResponse, InstallState, InstallStep, StepStatus } from "@/api/types";
import { StatusBadge } from "@/components/status";
import { toast } from "@/lib/toast";

/** Steps the terminal runs; everything after them is the browser onboarding. */
const TERMINAL = new Set(["preflight", "panel", "firewall", "install_engine", "bootstrap_engine", "edge_tls"]);

const isDone = (s: StepStatus) => s === "done" || s === "skipped";
const isFinished = (s: InstallState) => s.steps.length > 0 && s.steps.every((x) => isDone(x.status));

/**
 * The overview's setup card while setup is not complete: how far it is, the
 * step that needs attention, and the way back into the onboarding wizard
 * (/console/onboarding), which is where every step is answered. Polls
 * GET /install while a step is running or waiting.
 */
export function InstallChecklist({
  initial,
  onComplete,
}: {
  initial: InstallState;
  onComplete: () => void;
  /** Kept for the overview's signature; the wizard creates the first mailbox now. */
  onMailboxCreated?: (r: FirstMailboxResponse) => void;
}) {
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

  const active = state.steps.some((x) => x.status === "running" || x.status === "waiting");
  useEffect(() => {
    if (!active) return;
    const t = window.setInterval(() => {
      if (document.visibilityState === "visible") void poll();
    }, 5000);
    return () => window.clearInterval(t);
  }, [active, poll]);

  const browser = state.steps.filter((s) => !TERMINAL.has(s.id) && s.id !== "done");
  const terminalLeft = state.steps.find((s) => TERMINAL.has(s.id) && !isDone(s.status));
  const doneCount = browser.filter((s) => isDone(s.status)).length;
  const current: InstallStep | undefined = terminalLeft ?? browser.find((s) => !isDone(s.status));

  return (
    <Card className="mb-6">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <p className="text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
          Setup · {doneCount} of {browser.length} done
        </p>
        {active && (
          <p className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
            <Loader2 aria-hidden className="h-3 w-3 animate-spin motion-reduce:animate-none" />
            Checking by itself
          </p>
        )}
      </div>
      <ol className="mt-3 flex gap-1" aria-label={`${doneCount} of ${browser.length} setup steps done`}>
        {browser.map((s) => (
          <li
            key={s.id}
            title={s.title}
            className={cn(
              "h-1.5 flex-1 rounded-full",
              isDone(s.status) ? "bg-foreground/70" : s === current ? "bg-torii" : "bg-foreground/10",
            )}
          />
        ))}
      </ol>
      {current && (
        <div className="mt-5 flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0 max-w-2xl">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="font-serif text-[22px] leading-tight tracking-tight">{current.title}</h2>
              {current.status === "waiting" && (
                <StatusBadge tone="warn" icon={Hourglass}>
                  Waiting for you
                </StatusBadge>
              )}
            </div>
            {current.detail && <p className="mt-1.5 text-sm leading-relaxed text-foreground/85 [overflow-wrap:anywhere]">{current.detail}</p>}
            {terminalLeft && (
              <p className="mt-2 text-xs text-muted-foreground">
                Run <code className="font-mono text-foreground/80">sudo yunori-mail install</code> on the server to finish this part.
              </p>
            )}
          </div>
          {!terminalLeft && (
            <Button asChild variant="torii" size="sm">
              <Link to="/console/onboarding">
                Continue setup <ArrowRight aria-hidden className="h-3.5 w-3.5" />
              </Link>
            </Button>
          )}
        </div>
      )}
    </Card>
  );
}
