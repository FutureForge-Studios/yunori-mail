import { useSyncExternalStore } from "react";
import { CheckCircle2, Info, X, XCircle } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";

/**
 * A deliberately tiny toast — the client uses sonner with card styling; this
 * matches that look (bottom-right, card surface, hairline border) without the
 * dependency. Announced through a polite live region; errors are assertive.
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

export function Toaster() {
  const items = useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => toasts,
  );
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex w-[min(24rem,calc(100vw-2rem))] flex-col gap-2">
      <div aria-live="polite" className="sr-only">
        {items.filter((t) => t.tone !== "error").map((t) => t.message).join(". ")}
      </div>
      <div aria-live="assertive" className="sr-only">
        {items.filter((t) => t.tone === "error").map((t) => t.message).join(". ")}
      </div>
      {items.map((t) => (
        <div
          key={t.id}
          className="pointer-events-auto flex animate-fade-in items-start gap-2.5 rounded-lg border border-border bg-card px-3.5 py-3 text-sm text-card-foreground shadow-lg motion-reduce:animate-none"
        >
          {t.tone === "success" && <CheckCircle2 aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-emerald-700 dark:text-emerald-400" />}
          {t.tone === "error" && <XCircle aria-hidden className={cn("mt-0.5 h-4 w-4 shrink-0", "text-torii dark:text-[hsl(4_80%_70%)]")} />}
          {t.tone === "info" && <Info aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />}
          <p className="min-w-0 flex-1 leading-snug">{t.message}</p>
          <button
            type="button"
            onClick={() => dismiss(t.id)}
            aria-label="Dismiss"
            className="rounded-sm text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
