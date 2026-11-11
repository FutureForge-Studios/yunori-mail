import type { ReactNode } from "react";
import { ArrowUpRight, Link2, PartyPopper } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import type { FirstMailboxResponse } from "@/api/types";
import { CopyButton } from "@/components/copy-button";
import { SecretReveal } from "@/components/secret-reveal";
import { fmtDateTime, openInYunoriUrl } from "@/lib/format";
import { Button } from "@/ui/button";
import { Card } from "@/ui/card";
import { useSession } from "../session";

/**
 * The end of onboarding: the first mailbox's one-time password (when it was
 * created here), and how to start using it — the Yunori client, any mail app,
 * the self-service portal. Stays until dismissed.
 */
export function SetupComplete({
  created,
  finished,
  onDismiss,
  dismissLabel = "Done",
  className,
}: {
  created: FirstMailboxResponse | null;
  finished: boolean;
  onDismiss: () => void;
  dismissLabel?: string;
  className?: string;
}) {
  const { session } = useSession();
  const { hostname, clientOrigin } = session.server;
  const address = created?.mailbox.address ?? null;

  return (
    <Card padded={false} className={cn("mb-4 overflow-hidden", className)} aria-labelledby="setup-complete-title">
      <div className="flex flex-col gap-4 border-b border-border/70 px-5 py-5 sm:flex-row sm:items-start sm:justify-between sm:px-6">
        <div className="flex min-w-0 items-start gap-4">
          <span aria-hidden className="grid h-10 w-10 shrink-0 place-items-center rounded-full border border-success/25 bg-success/10 text-success-ink">
            <PartyPopper className="h-[18px] w-[18px]" strokeWidth={1.75} />
          </span>
          <div className="min-w-0">
            <p className="text-xs font-medium text-muted-foreground">{finished ? "Setup complete" : "First mailbox"}</p>
            <h2 id="setup-complete-title" className="mt-1 font-serif text-[24px] leading-[1.15] tracking-[-0.01em] text-foreground [overflow-wrap:anywhere]">
              {finished ? "Your mail server is ready." : `${address} is ready.`}
            </h2>
            <p className="mt-1.5 max-w-xl text-[13px] leading-relaxed text-muted-foreground">
              {finished
                ? "Mail can reach this server, and it signs what it sends. Connect a mail app to start using it."
                : "Setup finishes the last checks by itself. You can already connect a mail app."}
            </p>
          </div>
        </div>
        <Button variant="outline" size="sm" onClick={onDismiss} className="shrink-0 self-start sm:ml-4">
          {dismissLabel}
        </Button>
      </div>

      <div className="space-y-5 px-5 py-5 sm:px-6">
        {created?.password && <SecretReveal label={`Password for ${created.mailbox.address}`} value={created.password} />}
        {created?.invite && (
          <div className="rounded-xl border border-border bg-muted/40 p-4">
            <p className="flex min-w-0 items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <Link2 aria-hidden className="h-3.5 w-3.5 shrink-0" />
              <span className="min-w-0 [overflow-wrap:anywhere]">Invite link for {created.mailbox.address}</span>
            </p>
            <div className="mt-2 flex flex-col gap-2 sm:flex-row sm:items-center">
              <code className="min-w-0 flex-1 select-all break-all rounded-lg border border-border bg-card px-3 py-2 font-mono text-[13px] text-foreground shadow-card">
                {created.invite.url}
              </code>
              <CopyButton value={created.invite.url} label="invite link" showText text="Copy link" className="self-start sm:self-auto" />
            </div>
            <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
              Whoever opens it chooses the mailbox's password. It works once, until {fmtDateTime(created.invite.expiresAt)}; share it privately.
            </p>
          </div>
        )}

        <div className="grid gap-4 lg:grid-cols-2">
          <section className="flex min-w-0 flex-col rounded-xl border border-border p-4">
            <h3 className="text-sm font-semibold text-foreground">Read it in Yunori</h3>
            <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">
              The Yunori client recognises this server and fills in every setting; you only type the password.
            </p>
            <div className="mt-3">
              {address ? (
                <Button asChild size="sm" variant="torii" className="h-auto min-h-8 max-w-full whitespace-normal py-1.5 text-left">
                  <a href={openInYunoriUrl(clientOrigin, address)} target="_blank" rel="noreferrer">
                    <span className="min-w-0 [overflow-wrap:anywhere]">Open {address} in Yunori</span> <ArrowUpRight aria-hidden className="h-3.5 w-3.5" />
                  </a>
                </Button>
              ) : (
                <Button asChild size="sm" variant="outline">
                  <a href={clientOrigin} target="_blank" rel="noreferrer">
                    Open Yunori <ArrowUpRight aria-hidden className="h-3.5 w-3.5" />
                  </a>
                </Button>
              )}
            </div>
            <p className="mt-3 text-xs leading-relaxed text-muted-foreground">
              Each mailbox can change its password and create app passwords at{" "}
              <a
                className="rounded-sm font-mono text-foreground/85 underline underline-offset-2 [overflow-wrap:anywhere] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
                href="/account"
              >
                {hostname}/account
              </a>
              .
            </p>
          </section>
          <section className="min-w-0 rounded-xl border border-border p-4">
            <h3 className="text-sm font-semibold text-foreground">Any other mail app</h3>
            <p className="mt-1 text-[13px] leading-relaxed text-muted-foreground">Thunderbird, Apple Mail and Outlook usually find these by themselves from the address.</p>
            <dl className="mt-2 divide-y divide-border/60">
              <Setting label="Incoming (IMAP)">
                <span className="font-mono">{hostname}:993</span> · SSL/TLS
              </Setting>
              <Setting label="Outgoing (SMTP)">
                <span className="font-mono">{hostname}:465</span> · SSL/TLS, or 587 · STARTTLS
              </Setting>
              <Setting label="Username">{address ? <span className="font-mono">{address}</span> : "the full email address"}</Setting>
            </dl>
          </section>
        </div>
      </div>
    </Card>
  );
}

/** A mail-app setting: a small label over its value (the card can be narrow). */
function Setting({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="py-2">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 min-w-0 text-[13px] text-foreground [overflow-wrap:anywhere]">{children}</dd>
    </div>
  );
}
