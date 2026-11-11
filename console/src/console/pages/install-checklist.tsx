import { useCallback, useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { ArrowRight, Hourglass, Loader2 } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { api } from "@/api/client";
import type { FirstMailboxResponse, InstallState, InstallStep, StepStatus } from "@/api/types";
import { StatusBadge } from "@/components/status";
import { Button } from "@/ui/button";
import { Card } from "@/ui/card";
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

  const title = current?.title ?? "Finishing setup";
  return (
    <Card padded={false} className="mb-6" aria-labelledby="setup-card-title">
      <div className="flex flex-col gap-4 px-5 py-5 sm:flex-row sm:items-start sm:justify-between sm:px-6">
        <div className="min-w-0 max-w-2xl">
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs font-medium text-muted-foreground">
            <span>Setup</span>
            <span aria-hidden>·</span>
            <span className="tabular-nums">
              {doneCount} of {browser.length} done
            </span>
            {active && (
              <>
                <span aria-hidden>·</span>
                <span className="inline-flex items-center gap-1.5 font-normal">
                  <Loader2 aria-hidden className="h-3 w-3 animate-spin motion-reduce:animate-none" />
                  Checking by itself
                </span>
              </>
            )}
          </p>
          <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1.5">
            <h2 id="setup-card-title" className="font-serif text-[22px] leading-tight tracking-[-0.01em] text-foreground">
              {current ? <span className="sr-only">Next: </span> : null}
              {title}
            </h2>
            {current?.status === "waiting" && (
              <StatusBadge tone="warn" icon={Hourglass}>
                Waiting for you
              </StatusBadge>
            )}
          </div>
          {current?.detail && <p className="mt-1.5 text-[13px] leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">{current.detail}</p>}
          {terminalLeft && (
            <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
              Run <code className="font-mono text-foreground/85">sudo yunori-mail install</code> on the server to finish this part.
            </p>
          )}
        </div>
        {!terminalLeft && current && (
          <Button asChild variant="torii" className="w-full shrink-0 sm:w-auto">
            <Link to="/console/onboarding">
              Continue setup <ArrowRight aria-hidden className="h-3.5 w-3.5" />
            </Link>
          </Button>
        )}
      </div>
      <div className="rounded-b-xl border-t border-border/70 bg-muted/25 px-5 py-4 sm:px-6">
        <ol className="grid gap-1.5" style={{ gridTemplateColumns: `repeat(${Math.max(browser.length, 1)}, minmax(0, 1fr))` }} aria-label={`${doneCount} of ${browser.length} setup steps done`}>
          {browser.map((s) => {
            const here = s === current;
            const state = isDone(s.status) ? "done" : here ? "current step" : "not yet";
            return (
              <li key={s.id} className="min-w-0" aria-current={here ? "step" : undefined}>
                <span
                  aria-hidden
                  className={cn("block h-1.5 rounded-full", isDone(s.status) ? "bg-foreground/70" : here ? "bg-torii" : "bg-foreground/10")}
                />
                <span aria-hidden className={cn("mt-2 hidden truncate pr-2 text-2xs lg:block", here ? "font-medium text-foreground" : "text-muted-foreground")} title={s.title}>
                  {s.title}
                </span>
                <span className="sr-only">
                  {s.title}: {state}
                </span>
              </li>
            );
          })}
        </ol>
      </div>
    </Card>
  );
}
