import { cn } from "@/design/vendor/lib/utils";

/**
 * The Yunori mark, as the live product draws it (yunori.email
 * `src/components/logo.tsx`): the sun resting on its horizon. It takes the
 * current text colour, so it reads as ink on paper and as paper on night; the
 * accent version lives only in the favicon (public/favicon.svg).
 */
export function YunoriMark({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={cn("h-5 w-5 text-foreground", className)}
      aria-hidden
    >
      <path d="M8 19 A8 8 0 0 1 24 19 Z" fill="currentColor" />
      <rect x="4" y="22" width="24" height="2.6" rx="1.3" fill="currentColor" opacity="0.55" />
    </svg>
  );
}
