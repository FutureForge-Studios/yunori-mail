import { ArrowUpRight, PartyPopper } from "lucide-react";
import { Button } from "@/design/vendor/ui/button";
import { Card, KV, SectionLabel } from "@/design/vendor/admin/ui";
import type { FirstMailboxResponse } from "@/api/types";
import { SecretReveal } from "@/components/secret-reveal";
import { openInYunoriUrl } from "@/lib/format";
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
}: {
  created: FirstMailboxResponse | null;
  finished: boolean;
  onDismiss: () => void;
}) {
  const { session } = useSession();
  const { hostname, clientOrigin } = session.server;
  const address = created?.mailbox.address ?? null;

  return (
    <Card className="mb-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <SectionLabel>{finished ? "Setup complete" : "First mailbox"}</SectionLabel>
          <h2 className="-mt-1 flex items-center gap-2 font-serif text-[22px] leading-tight tracking-tight">
            <PartyPopper aria-hidden className="h-5 w-5 text-muted-foreground" strokeWidth={1.5} />
            {finished ? "Your mail server is ready." : `${address} is ready.`}
          </h2>
          <p className="mt-1 max-w-xl text-sm text-muted-foreground">
            {finished
              ? "Mail can reach this server, and it signs what it sends. Connect a mail app to start using it."
              : "Setup finishes the last checks by itself. You can already connect a mail app."}
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={onDismiss}>
          Done
        </Button>
      </div>

      {created && (
        <div className="mt-4">
          <SecretReveal label={`Password for ${created.mailbox.address}`} value={created.password} />
        </div>
      )}

      <div className="mt-4 grid gap-4 lg:grid-cols-2">
        <div>
          <SectionLabel>Read it in Yunori</SectionLabel>
          <p className="mb-3 text-xs leading-relaxed text-muted-foreground">
            The Yunori client recognises this server and fills in every setting; you only type the password.
          </p>
          {address ? (
            <Button asChild size="sm" variant="torii">
              <a href={openInYunoriUrl(clientOrigin, address)} target="_blank" rel="noreferrer">
                Open {address} in Yunori <ArrowUpRight aria-hidden className="h-3.5 w-3.5" />
              </a>
            </Button>
          ) : (
            <Button asChild size="sm" variant="outline">
              <a href={clientOrigin} target="_blank" rel="noreferrer">
                Open Yunori <ArrowUpRight aria-hidden className="h-3.5 w-3.5" />
              </a>
            </Button>
          )}
          <p className="mt-3 text-xs leading-relaxed text-muted-foreground">
            Each mailbox can change its password and create app passwords at{" "}
            <a className="font-mono text-foreground/85 underline underline-offset-2" href="/account">
              {hostname}/account
            </a>
            .
          </p>
        </div>
        <div>
          <SectionLabel>Any other mail app</SectionLabel>
          <p className="mb-1 text-xs leading-relaxed text-muted-foreground">
            Thunderbird, Apple Mail and Outlook usually find these by themselves from the address.
          </p>
          <KV label="Incoming (IMAP)">
            <span className="font-mono">{hostname}:993</span> · SSL/TLS
          </KV>
          <KV label="Outgoing (SMTP)">
            <span className="font-mono">{hostname}:465</span> · SSL/TLS, or 587 · STARTTLS
          </KV>
          <KV label="Username">{address ? <span className="font-mono">{address}</span> : "the full email address"}</KV>
        </div>
      </div>
    </Card>
  );
}
