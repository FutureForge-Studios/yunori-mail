import type { ReactNode } from "react";
import { ChevronLeft, ChevronRight, type LucideIcon } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { fmtNumber } from "@/lib/format";
import { IconButton } from "./button";
import { EmptyState, Skeleton } from "./feedback";

export interface Column<Row> {
  key: string;
  header: ReactNode;
  cell: (row: Row) => ReactNode;
  /** Classes for the table cell (and header). */
  className?: string;
  /** Hidden in the table below lg (still shown in phone cards). */
  hideBelowMd?: boolean;
  /**
   * How the column appears in the phone card layout (< md):
   * "title" — the card heading (default for the first column);
   * "aside" — top-right (default for a column keyed "actions");
   * "field" — label + value (default);  "hidden" — left out.
   */
  mobile?: "title" | "aside" | "field" | "hidden";
  /** Label in the phone card (defaults to `header` when it's a string). */
  mobileLabel?: string;
}

function placement<Row>(c: Column<Row>, i: number): NonNullable<Column<Row>["mobile"]> {
  if (c.mobile) return c.mobile;
  if (i === 0) return "title";
  if (c.key === "actions") return "aside";
  return "field";
}

/**
 * The console's table. A real <table> from md up; below md every row becomes
 * a card (title, aside, then label/value fields) so nothing scrolls sideways
 * on a phone. Loading skeletons and an honest empty state are built in.
 */
export function DataTable<Row>({
  columns,
  rows,
  rowKey,
  loading,
  emptyTitle,
  emptyHint,
  emptyIcon,
  emptyAction,
  onRowClick,
  footer,
  caption,
  className,
}: {
  columns: Column<Row>[];
  rows: Row[];
  rowKey: (row: Row) => string;
  loading?: boolean;
  emptyTitle?: string;
  emptyHint?: string;
  emptyIcon?: LucideIcon;
  emptyAction?: ReactNode;
  onRowClick?: (row: Row) => void;
  footer?: ReactNode;
  caption?: string;
  className?: string;
}) {
  if (loading) {
    return (
      <div className={cn("overflow-hidden rounded-xl border border-border bg-card shadow-card", className)} aria-busy="true">
        <Skeleton className="h-10 rounded-none" />
        <div className="divide-y divide-border/60">
          {Array.from({ length: 5 }).map((_, i) => (
            <div key={i} className="flex items-center gap-4 px-4 py-4">
              <Skeleton className="h-8 w-8 rounded-full" />
              <Skeleton className="h-3.5 flex-1" />
              <Skeleton className="hidden h-3.5 w-32 md:block" />
            </div>
          ))}
        </div>
      </div>
    );
  }
  if (rows.length === 0) {
    return <EmptyState icon={emptyIcon} title={emptyTitle ?? "Nothing here yet"} hint={emptyHint} action={emptyAction} className={className} />;
  }

  const titleCols = columns.filter((c, i) => placement(c, i) === "title");
  const asideCols = columns.filter((c, i) => placement(c, i) === "aside");
  const fieldCols = columns.filter((c, i) => placement(c, i) === "field");

  return (
    <div className={cn("overflow-hidden rounded-xl border border-border bg-card shadow-card", className)}>
      {/* md and up: a table */}
      <div className="hidden overflow-x-auto md:block">
        <table className="w-full text-sm">
          {caption && <caption className="sr-only">{caption}</caption>}
          <thead>
            <tr className="border-b border-border bg-muted/40">
              {columns.map((c) => (
                <th
                  key={c.key}
                  scope="col"
                  className={cn(
                    "h-10 whitespace-nowrap px-4 text-left text-xs font-medium text-muted-foreground first:pl-5 last:pr-5",
                    c.hideBelowMd && "hidden lg:table-cell",
                    c.className,
                  )}
                >
                  {c.header}
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="divide-y divide-border/60">
            {rows.map((row) => (
              <tr
                key={rowKey(row)}
                onClick={onRowClick ? () => onRowClick(row) : undefined}
                className={cn("transition-colors hover:bg-muted/30", onRowClick && "cursor-pointer focus-within:bg-muted/30")}
              >
                {columns.map((c) => (
                  <td key={c.key} className={cn("px-4 py-3 align-middle first:pl-5 last:pr-5", c.hideBelowMd && "hidden lg:table-cell", c.className)}>
                    {c.cell(row)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      {/* phones: one card per row */}
      <ul className="divide-y divide-border/60 md:hidden" aria-label={caption}>
        {rows.map((row) => (
          <li key={rowKey(row)} className="px-4 py-3.5" onClick={onRowClick ? () => onRowClick(row) : undefined}>
            <div className="flex items-start justify-between gap-3">
              <div className="min-w-0 flex-1 space-y-1">
                {titleCols.map((c) => (
                  <div key={c.key} className="min-w-0">
                    {c.cell(row)}
                  </div>
                ))}
              </div>
              {asideCols.length > 0 && (
                <div className="flex shrink-0 items-center gap-1">
                  {asideCols.map((c) => (
                    <div key={c.key}>{c.cell(row)}</div>
                  ))}
                </div>
              )}
            </div>
            {fieldCols.length > 0 && (
              <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-2.5">
                {fieldCols.map((c) => (
                  <div key={c.key} className="min-w-0">
                    <dt className="text-2xs font-medium uppercase tracking-[0.06em] text-muted-foreground">{c.mobileLabel ?? (typeof c.header === "string" ? c.header : c.key)}</dt>
                    <dd className="mt-0.5 min-w-0 break-words">{c.cell(row)}</dd>
                  </div>
                ))}
              </dl>
            )}
          </li>
        ))}
      </ul>
      {footer}
    </div>
  );
}

/** Paging footer for long tables. */
export function Pager({ page, pageSize, total, onPage }: { page: number; pageSize: number; total: number; onPage: (page: number) => void }) {
  const pages = Math.max(1, Math.ceil(total / pageSize));
  const from = total === 0 ? 0 : (page - 1) * pageSize + 1;
  const to = Math.min(total, page * pageSize);
  return (
    <div className="flex items-center justify-between border-t border-border bg-muted/25 px-4 py-2 sm:px-5">
      <span className="text-xs tabular-nums text-muted-foreground">
        {fmtNumber(from)}–{fmtNumber(to)} of {fmtNumber(total)}
      </span>
      <div className="flex items-center gap-1">
        <IconButton label="Previous page" disabled={page <= 1} onClick={() => onPage(page - 1)}>
          <ChevronLeft className="h-4 w-4" />
        </IconButton>
        <span className="min-w-14 text-center text-xs tabular-nums text-muted-foreground">
          {page} / {pages}
        </span>
        <IconButton label="Next page" disabled={page >= pages} onClick={() => onPage(page + 1)}>
          <ChevronRight className="h-4 w-4" />
        </IconButton>
      </div>
    </div>
  );
}

/** Toolbar row above a table: search + filters left, extras right. */
export function TableToolbar({ children, end, className }: { children: ReactNode; end?: ReactNode; className?: string }) {
  return (
    <div className={cn("mb-3 flex flex-col gap-2 sm:flex-row sm:flex-wrap sm:items-center", className)}>
      <div className="flex min-w-0 flex-1 flex-col gap-2 sm:flex-row sm:flex-wrap sm:items-center">{children}</div>
      {end && <div className="flex shrink-0 items-center gap-2">{end}</div>}
    </div>
  );
}
