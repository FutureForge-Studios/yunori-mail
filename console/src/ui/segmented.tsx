import { forwardRef, useId, type InputHTMLAttributes, type ReactNode } from "react";
import { Search, X } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Input } from "./form";

/**
 * Segmented control: one choice out of a few, shown at once. Radio-group
 * semantics (arrow keys move the choice), so it works for filters as well as
 * settings. Scrolls sideways inside itself if a phone is too narrow.
 */
export function Segmented<T extends string>({
  value,
  onChange,
  options,
  label,
  className,
  size = "md",
  fullWidth,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: ReactNode; count?: number }[];
  label: string;
  className?: string;
  size?: "sm" | "md";
  fullWidth?: boolean;
}) {
  const id = useId();
  const move = (i: number) => {
    const next = options[(i + options.length) % options.length];
    onChange(next.value);
    requestAnimationFrame(() => document.getElementById(`${id}-${next.value}`)?.focus());
  };
  return (
    <div className={cn("max-w-full overflow-x-auto", fullWidth && "w-full", className)}>
      <div
        role="radiogroup"
        aria-label={label}
        className={cn("inline-flex gap-0.5 rounded-lg border border-border bg-muted/60 p-0.5", fullWidth && "flex w-full")}
        onKeyDown={(e) => {
          const i = options.findIndex((o) => o.value === value);
          if (e.key === "ArrowRight" || e.key === "ArrowDown") {
            e.preventDefault();
            move(i + 1);
          } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
            e.preventDefault();
            move(i - 1);
          }
        }}
      >
        {options.map((o) => {
          const active = o.value === value;
          return (
            <button
              key={o.value}
              id={`${id}-${o.value}`}
              type="button"
              role="radio"
              aria-checked={active}
              tabIndex={active ? 0 : -1}
              onClick={() => onChange(o.value)}
              className={cn(
                "inline-flex shrink-0 items-center justify-center gap-1.5 whitespace-nowrap rounded-md font-medium transition-[background-color,color,box-shadow] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60",
                size === "sm" ? "h-7 px-2.5 text-xs" : "h-8 px-3 text-[13px]",
                fullWidth && "flex-1",
                active ? "bg-card text-foreground shadow-card ring-1 ring-border" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {o.label}
              {o.count !== undefined && (
                <span className={cn("rounded px-1 text-2xs tabular-nums", active ? "bg-muted text-foreground/80" : "text-muted-foreground")}>{o.count}</span>
              )}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** Search field with a leading icon and a clear button. */
export const SearchInput = forwardRef<HTMLInputElement, Omit<InputHTMLAttributes<HTMLInputElement>, "onChange" | "value"> & { value: string; onValueChange: (v: string) => void }>(
  ({ value, onValueChange, className, ...props }, ref) => (
    <div className={cn("relative w-full sm:w-72", className)}>
      <Search aria-hidden className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
      <Input ref={ref} type="search" value={value} onChange={(e) => onValueChange(e.target.value)} className="pl-9 pr-8 [&::-webkit-search-cancel-button]:hidden" {...props} />
      {value && (
        <button
          type="button"
          onClick={() => onValueChange("")}
          aria-label="Clear search"
          className="absolute right-1.5 top-1/2 grid h-6 w-6 -translate-y-1/2 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      )}
    </div>
  ),
);
SearchInput.displayName = "SearchInput";

/**
 * Underlined tabs for switching panels inside a card (devices, providers).
 * Same keyboard model as Segmented.
 */
export function Tabs<T extends string>({
  value,
  onChange,
  options,
  label,
  className,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: ReactNode }[];
  label: string;
  className?: string;
}) {
  const id = useId();
  const move = (i: number) => {
    const next = options[(i + options.length) % options.length];
    onChange(next.value);
    requestAnimationFrame(() => document.getElementById(`${id}-${next.value}`)?.focus());
  };
  return (
    <div className={cn("-mb-px max-w-full overflow-x-auto", className)}>
      <div
        role="tablist"
        aria-label={label}
        className="flex gap-1"
        onKeyDown={(e) => {
          const i = options.findIndex((o) => o.value === value);
          if (e.key === "ArrowRight") {
            e.preventDefault();
            move(i + 1);
          } else if (e.key === "ArrowLeft") {
            e.preventDefault();
            move(i - 1);
          }
        }}
      >
        {options.map((o) => {
          const active = o.value === value;
          return (
            <button
              key={o.value}
              id={`${id}-${o.value}`}
              type="button"
              role="tab"
              aria-selected={active}
              tabIndex={active ? 0 : -1}
              onClick={() => onChange(o.value)}
              className={cn(
                "relative shrink-0 whitespace-nowrap px-3 pb-3 pt-1 text-[13px] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/60",
                active ? "text-foreground" : "text-muted-foreground hover:text-foreground",
              )}
            >
              {o.label}
              <span aria-hidden className={cn("absolute inset-x-2 bottom-0 h-0.5 rounded-full", active ? "bg-torii" : "bg-transparent")} />
            </button>
          );
        })}
      </div>
    </div>
  );
}
