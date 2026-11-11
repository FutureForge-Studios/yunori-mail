import { useEffect, type ReactNode } from "react";
import { cn } from "@/design/vendor/lib/utils";

export function usePageTitle(title: string) {
  useEffect(() => {
    document.title = title ? `${title} · Yunori Mail` : "Yunori Mail";
  }, [title]);
}

const WIDTH = {
  /** Data pages: tables, dashboards. */
  wide: "max-w-[1480px]",
  /** Settings and forms. */
  default: "max-w-6xl",
  /** Single-column reading (portal, onboarding). */
  narrow: "max-w-3xl",
} as const;

/** The page container: gutters, max width, vertical rhythm. */
export function PageContainer({ width = "wide", className, children }: { width?: keyof typeof WIDTH; className?: string; children: ReactNode }) {
  return <div className={cn("mx-auto w-full px-4 pb-16 pt-6 sm:px-6 sm:pt-8 lg:px-8", WIDTH[width], className)}>{children}</div>;
}

/**
 * Page header: optional eyebrow, a Fraunces title, one line of context,
 * meta (badges) and actions. Actions wrap under the title on phones.
 */
export function PageHeader({
  title,
  blurb,
  eyebrow,
  meta,
  actions,
  className,
}: {
  title: ReactNode;
  blurb?: ReactNode;
  eyebrow?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <header className={cn("mb-6 flex flex-col gap-4 sm:mb-8 md:flex-row md:items-end md:justify-between", className)}>
      <div className="min-w-0">
        {eyebrow && <div className="mb-2 text-xs font-medium text-muted-foreground">{eyebrow}</div>}
        <h1 className="font-serif text-[26px] leading-[1.15] tracking-[-0.015em] text-foreground [overflow-wrap:anywhere] sm:text-[30px]">{title}</h1>
        {blurb && <p className="mt-2 max-w-3xl text-sm leading-relaxed text-muted-foreground">{blurb}</p>}
        {meta && <div className="mt-3 flex flex-wrap items-center gap-2">{meta}</div>}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </header>
  );
}

/** Container + header in one (the console's standard page). */
export function Page({
  title,
  blurb,
  eyebrow,
  meta,
  actions,
  width = "wide",
  children,
}: {
  title: ReactNode;
  blurb?: ReactNode;
  eyebrow?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  width?: keyof typeof WIDTH;
  children: ReactNode;
}) {
  return (
    <PageContainer width={width}>
      <PageHeader title={title} blurb={blurb} eyebrow={eyebrow} meta={meta} actions={actions} />
      {children}
    </PageContainer>
  );
}

/** A vertical stack of page sections with consistent spacing. */
export function Stack({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("space-y-6", className)}>{children}</div>;
}
