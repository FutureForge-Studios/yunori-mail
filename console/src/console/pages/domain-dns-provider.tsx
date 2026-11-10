import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, ExternalLink, Eye, RefreshCw, Unplug, UploadCloud, XCircle } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Card, Field, SectionLabel, Skeleton, selectClass } from "@/design/vendor/admin/ui";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { DnsPlan, DnsPlanAction, DnsPlanItem, DnsProviderStatus, DnsPublishResponse, Domain, DomainDnsResponse } from "@/api/types";
import { ConfirmAction } from "@/components/confirm";
import { FieldError } from "@/components/kit";
import { PasswordInput } from "@/components/password-input";
import { StatusBadge, type Tone } from "@/components/status";
import { timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";

const ACTION_META: Record<DnsPlanAction, { label: string; tone: Tone }> = {
  create: { label: "Add", tone: "ok" },
  update: { label: "Change", tone: "warn" },
  unchanged: { label: "Already right", tone: "muted" },
  manual: { label: "Needs you", tone: "bad" },
  skip: { label: "Left alone", tone: "muted" },
};

/**
 * "Publish automatically": connect a DNS provider (token entered once, never
 * shown again), preview exactly what would change, publish, then show the
 * public-resolver check. Yunori never deletes a record at the provider.
 */
export function DnsProviderCard({ domain, onChecked }: { domain: Domain; onChecked: (dns: DomainDnsResponse) => void }) {
  const [status, setStatus] = useState<DnsProviderStatus | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [editing, setEditing] = useState(false);
  const [plan, setPlan] = useState<DnsPlan | null>(null);
  const [result, setResult] = useState<DnsPublishResponse | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [disconnectOpen, setDisconnectOpen] = useState(false);

  useEffect(() => {
    let live = true;
    api
      .dnsProvider(domain.id)
      .then((s) => live && setStatus(s))
      .catch(() => live && setLoadError(true));
    return () => {
      live = false;
    };
  }, [domain.id]);

  async function preview() {
    setPreviewing(true);
    setResult(null);
    try {
      setPlan(await api.dnsPlan(domain.id));
    } catch (err) {
      toast.error(errorMessage(err));
    } finally {
      setPreviewing(false);
    }
  }

  if (loadError) {
    return (
      <Card>
        <SectionLabel>Publish automatically</SectionLabel>
        <p className="text-xs text-muted-foreground">The DNS provider settings could not be loaded. Reload the page to try again.</p>
      </Card>
    );
  }
  if (!status) return <Skeleton className="h-40" />;

  const conn = status.connected;
  return (
    <Card>
      <SectionLabel>Publish automatically</SectionLabel>
      {!conn || editing ? (
        <ConnectForm
          domain={domain}
          status={status}
          replacing={!!conn}
          onCancel={conn ? () => setEditing(false) : undefined}
          onConnected={(s) => {
            setStatus(s);
            setEditing(false);
            setPlan(null);
            setResult(null);
            toast.success(`${s.connected?.label ?? "DNS provider"} is connected for ${domain.name}.`);
          }}
        />
      ) : (
        <>
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm text-foreground/90">
                  {conn.label} · zone <span className="font-mono text-xs">{conn.zoneName}</span>
                </span>
                <StatusBadge tone="ok" icon={CheckCircle2}>
                  Connected
                </StatusBadge>
              </div>
              <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                Connected {timeAgo(conn.connectedAt)} by {conn.connectedBy}. The token is stored encrypted on this server and never shown again.
                {conn.lastPublish &&
                  ` Last published ${timeAgo(conn.lastPublish.at)}: ${conn.lastPublish.created} added, ${conn.lastPublish.updated} changed${
                    conn.lastPublish.failed ? `, ${conn.lastPublish.failed} failed` : ""
                  }.`}
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" onClick={() => void preview()} disabled={previewing}>
                {previewing ? <RefreshCw className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" /> : <Eye className="h-3.5 w-3.5" />}
                {previewing ? "Reading the zone…" : plan ? "Preview again" : "Preview changes"}
              </Button>
              <Button size="sm" variant="outline" onClick={() => setEditing(true)}>
                Replace token
              </Button>
              <Button size="sm" variant="ghost" onClick={() => setDisconnectOpen(true)}>
                <Unplug className="h-3.5 w-3.5" />
                Disconnect
              </Button>
            </div>
          </div>

          {plan && (
            <PlanView
              domain={domain}
              plan={plan}
              onStale={() => void preview()}
              onPublished={(r) => {
                setPlan(null);
                setResult(r);
                if (r.dns) onChecked(r.dns);
                setStatus((s) =>
                  s?.connected
                    ? { ...s, connected: { ...s.connected, lastPublish: { at: new Date().toISOString(), by: "you", created: r.created, updated: r.updated, failed: r.failed } } }
                    : s,
                );
              }}
              onCancel={() => setPlan(null)}
            />
          )}
          {result && <ResultView result={result} />}
        </>
      )}

      <ConfirmAction
        open={disconnectOpen}
        onOpenChange={setDisconnectOpen}
        title={`Disconnect ${conn?.label ?? "the DNS provider"}?`}
        description="Yunori forgets the token. Records already published stay exactly as they are; you can keep them up to date by hand or connect again later."
        confirmLabel="Disconnect"
        onConfirm={async () => {
          setStatus(await api.disconnectDnsProvider(domain.id));
          setPlan(null);
          setResult(null);
          toast.success("Disconnected. Published records were left as they are.");
        }}
      />
    </Card>
  );
}

// ── connect ──────────────────────────────────────────────────────────────────

function ConnectForm({
  domain,
  status,
  replacing,
  onCancel,
  onConnected,
}: {
  domain: Domain;
  status: DnsProviderStatus;
  replacing: boolean;
  onCancel?: () => void;
  onConnected: (s: DnsProviderStatus) => void;
}) {
  const [provider, setProvider] = useState(status.connected?.provider ?? status.providers[0]?.id ?? "");
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [tokenError, setTokenError] = useState<string | null>(null);
  const info = status.providers.find((p) => p.id === provider);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setTokenError(null);
    try {
      onConnected(await api.connectDnsProvider(domain.id, { provider, token: token.trim() || undefined }));
      setToken("");
    } catch (err) {
      if (err instanceof ApiRequestError && err.field === "token") setTokenError(err.message);
      else toast.error(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={(e) => void submit(e)} className="space-y-4">
      {!replacing && (
        <p className="-mt-1 text-xs leading-relaxed text-muted-foreground">
          Connect the DNS provider that hosts {domain.name} and Yunori writes the records below for you: only the ones that are missing or
          wrong, after you have seen every change. It never deletes a record, and every result is still checked against public DNS.
        </p>
      )}
      <div className="grid gap-4 sm:grid-cols-[12rem_1fr]">
        <Field label="DNS provider" htmlFor="dns-provider">
          <select id="dns-provider" className={selectClass} value={provider} onChange={(e) => setProvider(e.target.value)} disabled={status.providers.length < 2}>
            {status.providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
        </Field>
        <Field
          label="API token"
          htmlFor="dns-token"
          hint={
            <>
              {info?.tokenHelp}{" "}
              {info && (
                <a href={info.tokenUrl} target="_blank" rel="noreferrer" className="inline-flex items-center gap-0.5 underline underline-offset-2 hover:text-foreground">
                  Create a token
                  <ExternalLink aria-hidden className="h-3 w-3" />
                </a>
              )}
              {replacing ? " Leave it blank to re-check the saved token." : " Stored encrypted on this server and never shown again."}
            </>
          }
        >
          <PasswordInput
            id="dns-token"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            autoComplete="off"
            spellCheck={false}
            placeholder={replacing ? "Saved token (leave blank to keep)" : ""}
            aria-invalid={!!tokenError}
            aria-describedby={tokenError ? "dns-token-error" : undefined}
          />
          <FieldError id="dns-token-error">{tokenError}</FieldError>
        </Field>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="submit" size="sm" disabled={busy || (!replacing && !token.trim())}>
          {busy ? "Checking the token…" : replacing ? "Save and check" : `Connect ${info?.label ?? ""}`.trim()}
        </Button>
        {onCancel && (
          <Button type="button" size="sm" variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
        )}
        <span className="text-[11px] text-muted-foreground">Checked with read-only calls before it is saved.</span>
      </div>
    </form>
  );
}

// ── preview & publish ────────────────────────────────────────────────────────

function PlanView({
  domain,
  plan,
  onStale,
  onPublished,
  onCancel,
}: {
  domain: Domain;
  plan: DnsPlan;
  onStale: () => void;
  onPublished: (r: DnsPublishResponse) => void;
  onCancel: () => void;
}) {
  const changes = useMemo(() => plan.items.filter((i) => i.action === "create" || i.action === "update"), [plan]);
  const manual = plan.items.filter((i) => i.action === "manual");
  const quiet = plan.items.filter((i) => i.action === "unchanged" || i.action === "skip");
  const [selected, setSelected] = useState<Set<string>>(() => new Set(changes.map((i) => i.id)));
  const [busy, setBusy] = useState(false);
  useEffect(() => setSelected(new Set(changes.map((i) => i.id))), [changes]);

  async function publish() {
    setBusy(true);
    try {
      const all = selected.size === changes.length;
      const r = await api.publishDns(domain.id, { planId: plan.planId, only: all ? undefined : [...selected] });
      onPublished(r);
      const missing = r.dns?.records.filter((x) => x.required && x.status === "fail").length ?? 0;
      if (r.failed) toast.error(`${r.failed} record${r.failed === 1 ? "" : "s"} could not be published. See the details below.`);
      else
        toast.success(
          missing
            ? `Published ${r.created + r.updated} record${r.created + r.updated === 1 ? "" : "s"}. Public DNS can take a few minutes to catch up.`
            : `Published ${r.created + r.updated} record${r.created + r.updated === 1 ? "" : "s"}. Every required record resolves.`,
        );
    } catch (err) {
      if (err instanceof ApiRequestError && err.code === "plan_changed") {
        toast.info(err.message);
        onStale();
      } else toast.error(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  const toggle = (id: string, on: boolean) =>
    setSelected((s) => {
      const n = new Set(s);
      if (on) n.add(id);
      else n.delete(id);
      return n;
    });

  return (
    <div className="mt-5 border-t border-border/50 pt-4">
      <p className="text-xs text-muted-foreground" aria-live="polite">
        At {plan.providerLabel}, zone {plan.zoneName}: {plan.counts.create} to add, {plan.counts.update} to change, {plan.counts.unchanged} already right
        {plan.counts.manual ? `, ${plan.counts.manual} need${plan.counts.manual === 1 ? "s" : ""} you` : ""}. Nothing is deleted.
      </p>

      {changes.length === 0 ? (
        <p className="mt-3 rounded-md bg-muted/40 px-3 py-2 text-xs text-foreground/85">
          Nothing to publish: every record Yunori can write is already right at {plan.providerLabel}.
        </p>
      ) : (
        <ul className="mt-3 divide-y divide-border/40 overflow-hidden rounded-lg border border-border/60" aria-label="Changes to publish">
          {changes.map((i) => (
            <PlanRow key={i.id} item={i} checked={selected.has(i.id)} onToggle={(on) => toggle(i.id, on)} />
          ))}
        </ul>
      )}

      {manual.length > 0 && (
        <div className="mt-4">
          <p className="mb-2 text-xs font-medium text-foreground/85">Needs you at {plan.providerLabel}</p>
          <ul className="divide-y divide-border/40 overflow-hidden rounded-lg border border-border/60">
            {manual.map((i) => (
              <PlanRow key={i.id} item={i} />
            ))}
          </ul>
        </div>
      )}

      {quiet.length > 0 && (
        <details className="mt-4 text-xs">
          <summary className="cursor-pointer rounded-sm text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
            {plan.counts.unchanged} already right, {plan.counts.skip} left alone
          </summary>
          <ul className="mt-2 divide-y divide-border/40 overflow-hidden rounded-lg border border-border/60">
            {quiet.map((i) => (
              <PlanRow key={i.id} item={i} />
            ))}
          </ul>
        </details>
      )}

      <div className="mt-4 flex flex-wrap items-center gap-2">
        <Button size="sm" onClick={() => void publish()} disabled={busy || selected.size === 0}>
          <UploadCloud className="h-3.5 w-3.5" />
          {busy ? "Publishing…" : `Publish ${selected.size} change${selected.size === 1 ? "" : "s"}`}
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={busy}>
          Cancel
        </Button>
        <span className="text-[11px] text-muted-foreground">Then Yunori checks public DNS, exactly like “Check now”.</span>
      </div>
    </div>
  );
}

function PlanRow({ item: i, checked, onToggle }: { item: DnsPlanItem; checked?: boolean; onToggle?: (on: boolean) => void }) {
  const meta = ACTION_META[i.action];
  const inputId = `plan-${i.id}`;
  const body = (
    <div className="min-w-0 flex-1">
      <div className="flex flex-wrap items-center gap-2">
        <StatusBadge tone={meta.tone}>{meta.label}</StatusBadge>
        <span className="w-12 shrink-0 rounded border border-border/70 px-1 py-px text-center font-mono text-[10px] text-foreground/80">{i.type}</span>
        <span className="min-w-0 break-all font-mono text-xs text-foreground/90">{i.name}</span>
        {i.required && <span className="text-[11px] text-muted-foreground">Required</span>}
      </div>
      {(i.action === "update" || i.action === "manual") &&
        i.current.map((c, n) => (
          <code key={n} className="mt-1.5 block break-all font-mono text-[11px] text-muted-foreground line-through decoration-muted-foreground/50">
            {c}
          </code>
        ))}
      {i.action !== "skip" && (
        <code className={cn("mt-1 block break-all font-mono text-[11px] text-foreground/85", i.value.length > 140 && "line-clamp-2")}>{i.value}</code>
      )}
      {i.note && <p className="mt-1.5 text-[11px] leading-relaxed text-muted-foreground">{i.note}</p>}
    </div>
  );
  return (
    <li className="flex items-start gap-3 bg-card/30 px-3 py-2.5">
      {onToggle ? (
        <>
          <input
            id={inputId}
            type="checkbox"
            checked={checked}
            onChange={(e) => onToggle(e.target.checked)}
            className="mt-1 h-3.5 w-3.5 shrink-0 accent-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          />
          <label htmlFor={inputId} className="min-w-0 flex-1 cursor-pointer">
            {body}
          </label>
        </>
      ) : (
        body
      )}
    </li>
  );
}

function ResultView({ result: r }: { result: DnsPublishResponse }) {
  const failed = r.results.filter((x) => !x.ok);
  const name = (id: string) => r.items.find((i) => i.id === id);
  return (
    <div className="mt-5 border-t border-border/50 pt-4" aria-live="polite">
      <div className="flex flex-wrap items-center gap-2">
        <StatusBadge tone={failed.length ? "bad" : "ok"} icon={failed.length ? XCircle : CheckCircle2}>
          {failed.length ? "Partly published" : "Published"}
        </StatusBadge>
        <span className="text-xs text-foreground/85">
          {r.created} added, {r.updated} changed{failed.length ? `, ${failed.length} failed` : ""} at {r.providerLabel}.
        </span>
      </div>
      {failed.length > 0 && (
        <ul className="mt-3 space-y-1.5">
          {failed.map((f) => (
            <li key={f.id} className="rounded-md bg-muted/40 px-2.5 py-1.5 text-xs leading-relaxed">
              <span className="font-mono">
                {name(f.id)?.type} {name(f.id)?.name}
              </span>
              : {f.error}
            </li>
          ))}
        </ul>
      )}
      <p className="mt-2 text-[11px] leading-relaxed text-muted-foreground">
        The table below now shows what public resolvers return. New records usually appear within minutes; if one still shows as missing, use
        “Check now” again shortly.
      </p>
    </div>
  );
}
