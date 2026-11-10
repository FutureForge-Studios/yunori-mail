import { useState } from "react";
import { Ban, Plus, ShieldCheck, Trash2, Unlock } from "lucide-react";
import { Button } from "@/design/vendor/ui/button";
import { Input } from "@/design/vendor/ui/input";
import { Card, SectionLabel, Skeleton } from "@/design/vendor/admin/ui";
import { ApiRequestError, errorMessage } from "@/api/client";
import { accessApi, type AllowedAddress, type BanPolicy, type BlockedAddress } from "@/api/queue";
import { ConfirmAction } from "@/components/confirm";
import { FieldError, LoadError } from "@/components/kit";
import { StatusBadge } from "@/components/status";
import { fmtDateTime, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";

/** Loose client-side shape check; the server validates and canonicalises. */
const IP_OR_CIDR = /^(?:[0-9.]+|[0-9a-f:.]+)(?:\/\d{1,3})?$/i;

function duration(min: number): string {
  if (min % 1440 === 0) return `${min / 1440} day${min === 1440 ? "" : "s"}`;
  if (min % 60 === 0) return `${min / 60} hour${min === 60 ? "" : "s"}`;
  return `${min} minute${min === 1 ? "" : "s"}`;
}

function policySentence(p: BanPolicy): string {
  if (!p.enabled || p.failures == null || p.windowMinutes == null) return "Off: failed sign-ins never block an address.";
  return `${p.failures} failed sign-ins within ${duration(p.windowMinutes)} block the address ${p.banMinutes == null ? "until someone unblocks it" : `for ${duration(p.banMinutes)}`}.`;
}

/**
 * Login abuse: addresses the mail engine blocked after repeated failed sign-ins
 * (IMAP, SMTP, JMAP) or other abuse, and the allowlist that is never blocked.
 */
export function IpAccessSection() {
  const { data, error, reload } = useApi("security-access", accessApi.get);
  const [unblocking, setUnblocking] = useState<BlockedAddress | null>(null);
  const [removing, setRemoving] = useState<AllowedAddress | null>(null);

  return (
    <section className="mt-8" aria-labelledby="ip-access-title">
      <h2 id="ip-access-title" className="mb-3 text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground">
        Sign-in protection
      </h2>
      {error && !data ? (
        <LoadError error={error} onRetry={() => void reload()} what="Blocked addresses" />
      ) : !data ? (
        <div className="grid gap-4 lg:grid-cols-3">
          <Skeleton className="h-40 lg:col-span-2" />
          <Skeleton className="h-40" />
        </div>
      ) : (
        <div className="grid gap-4 lg:grid-cols-3">
          <Card className="lg:col-span-2">
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <SectionLabel>Blocked addresses</SectionLabel>
              {data.policy && (
                <span className="-mt-3 text-[11px] text-muted-foreground">
                  {data.policy.enabled ? (
                    <StatusBadge tone="ok" icon={ShieldCheck}>
                      Automatic blocking on
                    </StatusBadge>
                  ) : (
                    <StatusBadge tone="warn">Automatic blocking off</StatusBadge>
                  )}
                </span>
              )}
            </div>
            {data.policy && <p className="-mt-1 mb-3 text-xs leading-relaxed text-muted-foreground">{policySentence(data.policy)}</p>}
            {data.blocked.length === 0 ? (
              <p className="rounded-lg border border-dashed border-border/60 px-4 py-5 text-center text-xs text-muted-foreground">No address is blocked right now.</p>
            ) : (
              <ul className="divide-y divide-border/50" aria-label="Blocked addresses">
                {data.blocked.map((b) => (
                  <li key={b.id} className="flex flex-wrap items-center justify-between gap-2 py-2.5">
                    <div className="min-w-0">
                      <p className="flex items-center gap-2 font-mono text-xs text-foreground/90">
                        <Ban aria-hidden className="h-3.5 w-3.5 text-muted-foreground" />
                        {b.address}
                      </p>
                      <p className="mt-0.5 text-[11px] text-muted-foreground">
                        {b.reasonText}
                        {b.createdAt && (
                          <>
                            {" · "}
                            <time dateTime={b.createdAt} title={fmtDateTime(b.createdAt)}>
                              {timeAgo(b.createdAt)}
                            </time>
                          </>
                        )}
                        {" · "}
                        {b.expiresAt ? `until ${fmtDateTime(b.expiresAt)}` : "until unblocked"}
                      </p>
                    </div>
                    <Button size="sm" variant="ghost" onClick={() => setUnblocking(b)} aria-label={`Unblock ${b.address}`}>
                      <Unlock className="h-3.5 w-3.5" />
                      Unblock
                    </Button>
                  </li>
                ))}
              </ul>
            )}
            <p className="mt-3 border-t border-border/50 pt-3 text-[11px] leading-relaxed text-muted-foreground">
              The mail engine blocks an address on its own after repeated failed sign-ins, probing for addresses, or scanning for exploits. A
              person stuck behind a block is usually a phone or mail app still using an old password: unblock, then update the password on that device.
            </p>
          </Card>

          <Card>
            <SectionLabel>Never block</SectionLabel>
            <AllowForm
              onAdded={(a) => {
                toast.success(`${a.address} will never be blocked.`);
                void reload();
              }}
            />
            {data.allowed.length > 0 && (
              <ul className="mt-3 divide-y divide-border/50 border-t border-border/50" aria-label="Allowlist">
                {data.allowed.map((a) => (
                  <li key={a.id} className="flex items-center justify-between gap-2 py-2">
                    <div className="min-w-0">
                      <p className="truncate font-mono text-xs text-foreground/90">{a.address}</p>
                      {a.reason && <p className="truncate text-[11px] text-muted-foreground">{a.reason}</p>}
                    </div>
                    <button
                      type="button"
                      onClick={() => setRemoving(a)}
                      aria-label={`Remove ${a.address} from the allowlist`}
                      title="Remove"
                      className="grid h-8 w-8 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
              For your office or VPN. Keep it short: an allowlisted address can guess passwords without ever being blocked.
            </p>
          </Card>
        </div>
      )}

      <ConfirmAction
        open={!!unblocking}
        onOpenChange={(o) => !o && setUnblocking(null)}
        title={`Unblock ${unblocking?.address ?? ""}?`}
        description="It can connect and sign in again right away. If the failed sign-ins continue, it will be blocked again."
        confirmLabel="Unblock"
        onConfirm={async () => {
          const b = unblocking!;
          await accessApi.unblock(b.id);
          toast.success(`${b.address} is unblocked.`);
          void reload();
        }}
      />
      <ConfirmAction
        open={!!removing}
        onOpenChange={(o) => !o && setRemoving(null)}
        title={`Remove ${removing?.address ?? ""} from the allowlist?`}
        description="Failed sign-ins from it will count towards a block again."
        confirmLabel="Remove"
        destructive
        onConfirm={async () => {
          const a = removing!;
          await accessApi.removeAllowed(a.id);
          toast.success(`${a.address} was removed from the allowlist.`);
          void reload();
        }}
      />
    </section>
  );
}

function AllowForm({ onAdded }: { onAdded: (a: AllowedAddress) => void }) {
  const [address, setAddress] = useState("");
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    const value = address.trim();
    if (!IP_OR_CIDR.test(value)) {
      setErr("Enter an IP address (203.0.113.7) or a network (203.0.113.0/24).");
      return;
    }
    setBusy(true);
    setErr(null);
    try {
      const a = await accessApi.allow(value, note.trim());
      setAddress("");
      setNote("");
      onAdded(a);
    } catch (e2) {
      setErr(e2 instanceof ApiRequestError ? e2.message : errorMessage(e2));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} noValidate className="space-y-2">
      <Input
        value={address}
        onChange={(e) => setAddress(e.target.value)}
        placeholder="203.0.113.0/24"
        aria-label="IP address or network"
        aria-invalid={!!err}
        aria-describedby={err ? "allow-error" : undefined}
        autoComplete="off"
        spellCheck={false}
        autoCapitalize="none"
        className="h-9 font-mono text-xs"
      />
      <Input value={note} onChange={(e) => setNote(e.target.value)} placeholder="Note, e.g. office (optional)" aria-label="Note" maxLength={120} className="h-9 text-xs" />
      <FieldError id="allow-error">{err}</FieldError>
      <Button type="submit" size="sm" variant="outline" disabled={busy || !address.trim()} className="w-full">
        <Plus className="h-3.5 w-3.5" />
        {busy ? "Adding…" : "Add to allowlist"}
      </Button>
    </form>
  );
}
