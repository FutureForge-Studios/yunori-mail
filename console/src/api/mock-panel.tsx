import { useState } from "react";
import { FlaskConical, X } from "lucide-react";
import { SCENARIOS, currentScenario, setScenario, type Scenario } from "./mock";

/**
 * Dev-only scenario switcher, mounted by main.tsx in mock builds only.
 * Collapsed to a small pill in the bottom-left corner so it stays out of the way.
 */
export default function MockPanel() {
  const [open, setOpen] = useState(false);
  const active = currentScenario();

  const choose = (s: Scenario) => {
    setScenario(s);
    const target = SCENARIOS.find((x) => x.key === s)!;
    window.location.assign(target.path);
  };

  if (!open) {
    return (
      <button
        type="button"
        data-mock-panel
        onClick={() => setOpen(true)}
        className="fixed bottom-3 left-3 z-[70] inline-flex items-center gap-1.5 rounded-full border border-dashed border-border bg-card px-2.5 py-1 text-[11px] text-muted-foreground shadow-sm transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        aria-label={`Mock data: ${SCENARIOS.find((s) => s.key === active)?.label}. Change scenario`}
      >
        <FlaskConical className="h-3 w-3" aria-hidden />
        Mock · {SCENARIOS.find((s) => s.key === active)?.label}
      </button>
    );
  }

  return (
    <div
      data-mock-panel
      role="dialog"
      aria-label="Mock scenarios"
      className="fixed bottom-3 left-3 z-[70] w-80 rounded-xl border border-border bg-card p-3 text-xs shadow-lg"
    >
      <div className="mb-2 flex items-center justify-between">
        <p className="text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">Mock scenario</p>
        <button
          type="button"
          onClick={() => setOpen(false)}
          aria-label="Close"
          className="rounded-sm text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
      <ul className="space-y-1">
        {SCENARIOS.map((s) => (
          <li key={s.key}>
            <button
              type="button"
              onClick={() => choose(s.key)}
              aria-current={s.key === active}
              className={
                "w-full rounded-md px-2.5 py-1.5 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring " +
                (s.key === active ? "bg-foreground/[0.06] text-foreground" : "text-muted-foreground hover:bg-accent hover:text-foreground")
              }
            >
              <span className="block text-[13px]">{s.label}</span>
              <span className="block text-[11px] leading-snug text-muted-foreground">{s.hint}</span>
            </button>
          </li>
        ))}
      </ul>
      <p className="mt-2 border-t border-border/60 pt-2 text-[11px] leading-snug text-muted-foreground">
        Also: <code className="font-mono">?mock=healthy</code> on any URL. Portal: any mailbox + any 8+ char password.
      </p>
    </div>
  );
}
