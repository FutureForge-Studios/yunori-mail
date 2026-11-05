import { cn } from "@/design/vendor/lib/utils";

/**
 * Quiet usage bar. The visible text carries the meaning; the bar tints amber
 * past 90% (with the words "nearly full" alongside — never color alone).
 */
export function Meter({
  value,
  label,
  valueText,
  className,
}: {
  /** 0–100, or null when unknown/unlimited. */
  value: number | null;
  label: string;
  valueText: string;
  className?: string;
}) {
  const high = value !== null && value >= 90;
  return (
    <div className={className}>
      <div
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={value === null ? undefined : Math.round(value)}
        aria-valuetext={valueText}
        className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
      >
        {value !== null && (
          <div
            className={cn(
              "h-full rounded-full transition-[width] duration-500 motion-reduce:transition-none",
              high ? "bg-amber-600 dark:bg-amber-400" : "bg-foreground/60",
            )}
            style={{ width: `${Math.max(value, value > 0 ? 2 : 0)}%` }}
          />
        )}
      </div>
    </div>
  );
}
