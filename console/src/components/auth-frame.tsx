import type { ReactNode } from "react";
import { ToriLogo } from "@/design/vendor/logo";
import { ThemeToggle } from "./theme-toggle";

/**
 * The calm single-column frame the client uses for sign-in: logo, serif
 * headline, one quiet line under it, the form, a footnote.
 */
export function AuthFrame({
  eyebrow,
  title,
  subtitle,
  children,
  footnote,
  width = "max-w-sm",
}: {
  eyebrow?: ReactNode;
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
  footnote?: ReactNode;
  width?: string;
}) {
  return (
    <div className="relative grid min-h-dvh place-items-center bg-background px-6 py-12">
      <div className="absolute right-3 top-3">
        <ThemeToggle />
      </div>
      <main className={`w-full ${width} animate-fade-in motion-reduce:animate-none`}>
        <div className="mb-10 flex flex-col items-center gap-3">
          <ToriLogo className="h-9 w-9" />
          <div className="text-center">
            {eyebrow && <p className="mb-2 text-[11px] uppercase tracking-[0.18em] text-muted-foreground">{eyebrow}</p>}
            <h1 className="font-serif text-3xl tracking-tight">{title}</h1>
            {subtitle && <p className="mt-1 text-sm text-muted-foreground">{subtitle}</p>}
          </div>
        </div>
        {children}
        {footnote && <p className="mt-8 text-center text-xs leading-relaxed text-muted-foreground">{footnote}</p>}
      </main>
    </div>
  );
}

/** Form-level error (non-field), announced. */
export function FormError({ children }: { children: ReactNode }) {
  if (!children) return null;
  return (
    <p role="alert" className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs leading-relaxed text-foreground/90">
      {children}
    </p>
  );
}
