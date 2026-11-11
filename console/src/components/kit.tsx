import type { ReactNode } from "react";
import { Segmented } from "@/ui/segmented";

/**
 * Compatibility barrel: console pages (and the onboarding screens) import
 * their building blocks from here. Everything now comes from the console's
 * own kit in src/ui (see src/ui/README.md), which composes the vendored
 * client primitives.
 */
export { Page, PageHeader, PageContainer, usePageTitle } from "@/ui/page";
export { Card, CardBody, CardFooter, CardHeader, KV, KVList, SectionHeader, SectionLabel } from "@/ui/card";
export { Chip } from "@/ui/badge";
export { EmptyState, LoadError, Skeleton } from "@/ui/feedback";
export { Field, FieldError, RadioRow, selectClass } from "@/ui/form";
export { StatCard } from "@/ui/data-viz";
export { DataTable, Pager, type Column } from "@/ui/data-table";

/** Filter control (kept for callers): a segmented radio group. */
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
  return <Segmented value={value} onChange={onChange} options={options} label={label} />;
}

/** Same as FilterPills, with optional counts. */
export function SegmentedTabs<T extends string>(props: {
  value: T;
  onChange: (v: T) => void;
  tabs: { value: T; label: ReactNode; count?: number }[];
  label: string;
  className?: string;
}) {
  return <Segmented value={props.value} onChange={props.onChange} options={props.tabs} label={props.label} className={props.className} />;
}
