import type { ReactNode } from "react";
import { KeyRound } from "lucide-react";
import { CopyButton } from "./copy-button";

/**
 * One-time secret display (generated mailbox passwords, app passwords). The
 * server never shows it again, so the copy says so plainly.
 */
export function SecretReveal({
  label,
  value,
  note = "Shown once. The server keeps no readable copy — store it somewhere safe now.",
  children,
}: {
  label: string;
  value: string;
  note?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="rounded-xl border border-border bg-muted/40 p-4">
      <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <KeyRound aria-hidden className="h-3.5 w-3.5" />
        {label}
      </p>
      <div className="mt-2 flex flex-col gap-2 sm:flex-row sm:items-center">
        <code
          className="min-w-0 flex-1 select-all break-all rounded-lg border border-border bg-card px-3 py-2 font-mono text-[15px] tracking-wide text-foreground shadow-card"
          aria-label={label}
        >
          {value}
        </code>
        <CopyButton value={value} label={label} showText />
      </div>
      <p className="mt-2 text-xs leading-relaxed text-muted-foreground">{note}</p>
      {children}
    </div>
  );
}
