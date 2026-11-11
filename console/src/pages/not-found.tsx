import { Link } from "react-router-dom";
import { Compass } from "lucide-react";
import { YunoriMark } from "@/components/yunori-logo";
import { usePageTitle } from "@/components/kit";
import { Button } from "@/ui/button";

/** Honest 404 — inside the console shell, or standalone. */
export function NotFound({ inShell }: { inShell?: boolean }) {
  usePageTitle("Not found");
  const body = (
    <div className="max-w-sm text-center">
      {inShell ? (
        <span aria-hidden className="mx-auto grid h-12 w-12 place-items-center rounded-full border border-border bg-card text-icon shadow-card">
          <Compass className="h-5 w-5" strokeWidth={1.75} />
        </span>
      ) : (
        <YunoriMark className="mx-auto h-9 w-9" />
      )}
      <p className="mt-5 text-xs font-semibold uppercase tracking-[0.14em] text-muted-foreground">404</p>
      <h1 className="mt-2 font-serif text-3xl tracking-[-0.02em]">Nothing lives here</h1>
      <p className="mt-2 text-sm text-muted-foreground">The address may be mistyped, or the page has moved.</p>
      <div className="mt-6 flex justify-center gap-2">
        <Button asChild variant={inShell ? "default" : "outline"} size="sm">
          <Link to="/console">Server console</Link>
        </Button>
        {!inShell && (
          <Button asChild variant="outline" size="sm">
            <Link to="/account">Your mailbox</Link>
          </Button>
        )}
      </div>
    </div>
  );
  return inShell ? <div className="grid min-h-[60vh] place-items-center px-6">{body}</div> : <div className="grid min-h-dvh place-items-center bg-background px-6">{body}</div>;
}
