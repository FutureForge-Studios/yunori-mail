import type { ReactNode } from "react";
import { Lock, ShieldCheck, Sparkles } from "lucide-react";
import { YunoriMark } from "@/components/yunori-logo";
import { FormError } from "@/ui/feedback";
import { ThemeToggle } from "./theme-toggle";

/**
 * Sign-in and invitation frame. On large screens a night-ink brand panel sits
 * beside the form (it always uses the dark tokens, in either theme); on
 * phones it collapses to the mark above a single calm column.
 */
export function AuthFrame({
  eyebrow,
  title,
  subtitle,
  children,
  footnote,
  width = "max-w-sm",
  aside,
}: {
  eyebrow?: ReactNode;
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
  footnote?: ReactNode;
  width?: string;
  /** Replaces the brand panel's default copy. */
  aside?: ReactNode;
}) {
  return (
    <div className="grid min-h-dvh bg-background lg:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)] xl:grid-cols-[minmax(0,5fr)_minmax(0,7fr)]">
      <BrandPanel>{aside}</BrandPanel>
      <div className="relative flex min-h-dvh flex-col">
        <div className="flex items-center justify-between px-4 pt-4 sm:px-6">
          <span className="flex items-center gap-2 lg:invisible">
            <YunoriMark className="h-6 w-6" />
            <span className="text-sm font-semibold tracking-tight">Yunori Mail</span>
          </span>
          <ThemeToggle />
        </div>
        <main className="grid flex-1 place-items-center px-5 py-10 sm:px-8">
          <div className={`w-full ${width} animate-rise-in motion-reduce:animate-none`}>
            <div className="mb-8">
              {eyebrow && <p className="mb-3 text-xs font-medium uppercase tracking-[0.14em] text-muted-foreground">{eyebrow}</p>}
              <h1 className="font-serif text-[32px] leading-[1.1] tracking-[-0.02em]">{title}</h1>
              {subtitle && <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{subtitle}</p>}
            </div>
            {children}
            {footnote && <p className="mt-8 border-t border-border pt-5 text-xs leading-relaxed text-muted-foreground">{footnote}</p>}
          </div>
        </main>
      </div>
    </div>
  );
}

function BrandPanel({ children }: { children?: ReactNode }) {
  return (
    <aside className="dark brand-wash relative hidden overflow-hidden border-r border-border text-foreground lg:flex lg:flex-col lg:justify-between lg:p-10 xl:p-14">
      <div className="flex items-center gap-2.5">
        <YunoriMark className="h-7 w-7" />
        <span className="text-[15px] font-semibold tracking-tight">Yunori Mail</span>
      </div>
      <div className="relative max-w-md">
        {children ?? (
          <>
            <p className="font-serif text-[34px] leading-[1.12] tracking-[-0.02em]">Your mail, on your server — calm, private and yours.</p>
            <ul className="mt-8 space-y-3.5 text-sm text-muted-foreground">
              <li className="flex items-start gap-3">
                <ShieldCheck aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-torii-ink" />
                DNS, certificates and reputation checked for real, against public resolvers.
              </li>
              <li className="flex items-start gap-3">
                <Lock aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-torii-ink" />
                Secrets are sealed on the server and never shown twice.
              </li>
              <li className="flex items-start gap-3">
                <Sparkles aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-torii-ink" />
                Every change an admin makes is written to a tamper-evident ledger.
              </li>
            </ul>
          </>
        )}
      </div>
      <p className="relative text-xs text-muted-foreground">Self-hosted · Stalwart inside · yunori.email</p>
    </aside>
  );
}

export { FormError };
