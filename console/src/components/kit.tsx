import { useEffect, type ReactNode } from "react";
import { RotateCw } from "lucide-react";
import { Button } from "@/design/vendor/ui/button";
import { EmptyState } from "@/design/vendor/admin/ui";
import { errorMessage } from "@/api/client";

/** The client's admin kit (vendored) — one import site for the console. */
export {
  AdminPage as Page,
  Card,
  Chip,
  EmptyState,
  Field,
  KV,
  SectionLabel,
  SegmentedTabs,
  Skeleton,
  StatCard,
  selectClass,
} from "@/design/vendor/admin/ui";
export { DataTable, type Column } from "@/design/vendor/admin/data-table";

export function usePageTitle(title: string) {
  useEffect(() => {
    document.title = title ? `${title} · Yunori Mail` : "Yunori Mail";
  }, [title]);
}

/** Honest load failure with a retry — never a blank page. */
export function LoadError({ error, onRetry, what }: { error: unknown; onRetry: () => void; what: string }) {
  return (
    <EmptyState
      title={`${what} couldn't load`}
      hint={errorMessage(error, "The server didn't answer. Check that yunori-mail is running.")}
      action={
        <Button variant="outline" size="sm" onClick={onRetry}>
          <RotateCw className="h-3.5 w-3.5" />
          Try again
        </Button>
      }
    />
  );
}

/** Quiet pill-style filter group (the client's admin filter pattern). */
export function FilterPills<T extends string>({
  value,
  onChange,
  options,
  label,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: ReactNode }[];
  label: string;
}) {
  return (
    <div className="flex flex-wrap gap-1" role="group" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          aria-pressed={value === o.value}
          className={
            value === o.value
              ? "rounded-full border border-foreground/30 bg-foreground/5 px-3 py-1.5 text-xs text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              : "rounded-full border border-border/60 px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          }
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Inline form error tied to a field via aria-describedby. */
export function FieldError({ id, children }: { id: string; children: ReactNode }) {
  if (!children) return null;
  return (
    <p id={id} role="alert" className="text-[11px] leading-relaxed text-torii dark:text-[hsl(4_80%_70%)]">
      {children}
    </p>
  );
}
