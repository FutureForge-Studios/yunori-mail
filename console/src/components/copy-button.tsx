import { useEffect, useState } from "react";
import { Check, Copy } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { copyText } from "@/lib/clipboard";
import { toast } from "@/lib/toast";

/** Icon (or labelled) copy button with a polite "Copied" announcement. */
export function CopyButton({
  value,
  label,
  showText,
  text = "Copy",
  className,
}: {
  value: string;
  /** What is being copied, for the accessible name ("Copy MX value"). */
  label: string;
  showText?: boolean;
  /** Button text when `showText` (default "Copy"). */
  text?: string;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(t);
  }, [copied]);

  return (
    <button
      type="button"
      onClick={async () => {
        if (await copyText(value)) setCopied(true);
        else toast.error("Couldn't copy — select the text and copy it manually.");
      }}
      aria-label={showText ? undefined : `Copy ${label}`}
      title={showText ? undefined : `Copy ${label}`}
      className={cn(
        "inline-flex shrink-0 items-center gap-1.5 rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
        showText ? "h-8 border border-border px-2.5 text-xs shadow-sm" : "h-7 w-7 justify-center",
        className,
      )}
    >
      {copied ? <Check aria-hidden className="h-3.5 w-3.5" /> : <Copy aria-hidden className="h-3.5 w-3.5" />}
      {showText && (copied ? "Copied" : text)}
      <span className="sr-only" aria-live="polite">
        {copied ? `${label} copied` : ""}
      </span>
    </button>
  );
}
