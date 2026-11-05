import { Link } from "react-router-dom";
import { ToriLogo } from "@/design/vendor/logo";
import { usePageTitle } from "@/components/kit";

/** Honest 404 — inside the console shell, or standalone. */
export function NotFound({ inShell }: { inShell?: boolean }) {
  usePageTitle("Not found");
  const body = (
    <div className="max-w-sm text-center">
      {!inShell && <ToriLogo className="mx-auto h-8 w-8 opacity-70" />}
      <p className="mt-5 text-[11px] uppercase tracking-[0.18em] text-muted-foreground">404</p>
      <h1 className="mt-2 font-serif text-3xl tracking-tight">Nothing lives here</h1>
      <p className="mt-2 text-sm text-muted-foreground">The address may be mistyped, or the page has moved.</p>
      <div className="mt-6 flex justify-center gap-3">
        <Link
          to="/console"
          className="rounded-md border border-border px-4 py-2 text-sm shadow-sm transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          Server console
        </Link>
        {!inShell && (
          <Link
            to="/account"
            className="rounded-md border border-border px-4 py-2 text-sm shadow-sm transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          >
            Your mailbox
          </Link>
        )}
      </div>
    </div>
  );
  return inShell ? <div className="grid min-h-[60vh] place-items-center px-6">{body}</div> : <div className="grid min-h-dvh place-items-center bg-background px-6">{body}</div>;
}
