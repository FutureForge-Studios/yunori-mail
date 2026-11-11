import { useSyncExternalStore } from "react";
import { CheckCircle2, Info, X, XCircle } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";

/**
 * A deliberately tiny toast: bottom-right on desktop, bottom-centre on
 * phones, a card with a tone stripe. Announced through a polite live region;
 * errors are assertive and stay longer.
 */
type Tone = "success" | "error" | "info";
interface Toast {
  id: number;
  tone: Tone;
  message: string;
}

let toasts: Toast[] = [];
let seq = 0;
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

function push(tone: Tone, message: string, ms = tone === "error" ? 7000 : 4000) {
  const t = { id: ++seq, tone, message };
  toasts = [...toasts.slice(-3), t];
  emit();
  window.setTimeout(() => dismiss(t.id), ms);
}

function dismiss(id: number) {
  toasts = toasts.filter((t) => t.id !== id);
  emit();
}

export const toast = {
  success: (m: string) => push("success", m),
  error: (m: string) => push("error", m),
  info: (m: string) => push("info", m),
};

const ICON = { success: CheckCircle2, error: XCircle, info: Info } as const;
const INK = { success: "text-success-ink", error: "text-danger-ink", info: "text-info-ink" } as const;
const STRIPE = { success: "bg-success", error: "bg-destructive", info: "bg-info" } as const;

export function Toaster() {
  const items = useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => toasts,
  );
  return (
    <div className="pointer-events-none fixed inset-x-3 bottom-3 z-[60] flex flex-col items-center gap-2 sm:inset-x-auto sm:bottom-5 sm:right-5 sm:w-[24rem] sm:items-stretch">
      <div aria-live="polite" className="sr-only">
        {items.filter((t) => t.tone !== "error").map((t) => t.message).join(". ")}
      </div>
      <div aria-live="assertive" className="sr-only">
        {items.filter((t) => t.tone === "error").map((t) => t.message).join(". ")}
      </div>
      {items.map((t) => {
        const Icon = ICON[t.tone];
        return (
          <div
            key={t.id}
            className="pointer-events-auto relative flex w-full animate-toast-in items-start gap-3 overflow-hidden rounded-xl border border-border bg-popover py-3 pl-4 pr-3 text-sm text-popover-foreground shadow-pop motion-reduce:animate-none"
          >
            <span aria-hidden className={cn("absolute inset-y-0 left-0 w-1", STRIPE[t.tone])} />
            <Icon aria-hidden className={cn("mt-0.5 h-4 w-4 shrink-0", INK[t.tone])} />
            <p className="min-w-0 flex-1 leading-snug">{t.message}</p>
            <button
              type="button"
              onClick={() => dismiss(t.id)}
              aria-label="Dismiss"
              className="-m-1 grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
