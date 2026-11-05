import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Check as CheckIcon, Dot, ExternalLink } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Input } from "@/design/vendor/ui/input";
import { Card, SectionLabel, Skeleton } from "@/design/vendor/admin/ui";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { RelayProvider, SendingResponse } from "@/api/types";
import { FieldError, LoadError, Page, selectClass, usePageTitle } from "@/components/kit";
import { CheckList, StatusBadge } from "@/components/status";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";
import { useShell } from "../shell";

type Mode = "direct" | "relay";
interface RelayForm {
  provider: string;
  host: string;
  port: string;
  username: string;
  password: string;
}

const emptyRelay = (p?: RelayProvider): RelayForm => ({ provider: p?.key ?? "", host: p?.host ?? "", port: String(p?.port ?? 587), username: "", password: "" });

/** Direct vs relay, with the consequences of each spelled out. */
export default function SendingPage() {
  usePageTitle("Sending");
  const { data, error, reload, setData } = useApi("sending", api.sending);

  if (error && !data) {
    return (
      <Page title="Sending">
        <LoadError error={error} onRetry={() => void reload()} what="Sending settings" />
      </Page>
    );
  }
  if (!data) {
    return (
      <Page title="Sending" blurb="How mail leaves this server.">
        <div className="grid gap-4 md:grid-cols-2">
          <Skeleton className="h-56" />
          <Skeleton className="h-56" />
        </div>
        <Skeleton className="mt-4 h-40" />
      </Page>
    );
  }
  return <SendingForm data={data} onSaved={setData} />;
}

function SendingForm({ data, onSaved }: { data: SendingResponse; onSaved: (d: SendingResponse) => void }) {
  const { refreshHealth } = useShell();
  const [mode, setMode] = useState<Mode>(data.mode);
  const [relay, setRelay] = useState<RelayForm>(() =>
    data.relay
      ? { provider: data.relay.provider, host: data.relay.host, port: String(data.relay.port), username: data.relay.username, password: "" }
      : emptyRelay(data.providers[0]),
  );
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});

  useEffect(() => setMode(data.mode), [data.mode]);

  const blocked = data.port25.status === "fail";
  const provider = data.providers.find((p) => p.key === relay.provider);
  const dirty = useMemo(() => {
    if (mode !== data.mode) return true;
    if (mode === "direct") return false;
    const r = data.relay;
    return !r || r.provider !== relay.provider || r.host !== relay.host || String(r.port) !== relay.port || r.username !== relay.username || relay.password !== "";
  }, [mode, relay, data]);

  const set = (k: keyof RelayForm, v: string) => setRelay((r) => ({ ...r, [k]: v }));
  const chooseProvider = (key: string) => {
    const p = data.providers.find((x) => x.key === key);
    setRelay((r) => ({ ...r, provider: key, host: p?.host ?? (r.provider === key ? r.host : ""), port: String(p?.port ?? 587) }));
  };

  async function save(e: React.FormEvent) {
    e.preventDefault();
    const errs: Record<string, string> = {};
    if (mode === "relay") {
      if (!relay.provider) errs.provider = "Choose a relay provider.";
      if (!relay.host.trim()) errs.host = "Enter the relay's SMTP host.";
      const port = Number(relay.port);
      if (!Number.isInteger(port) || port < 1 || port > 65535) errs.port = "Between 1 and 65535.";
      if (!relay.username.trim()) errs.username = "Enter the relay username.";
      if (!relay.password && !data.relay?.hasPassword) errs.password = "Enter the relay password.";
    }
    setErrors(errs);
    if (Object.keys(errs).length) return;
    setBusy(true);
    try {
      await api.updateSending(
        mode === "relay"
          ? {
              mode,
              relay: {
                provider: relay.provider,
                host: relay.host.trim(),
                port: Number(relay.port),
                username: relay.username.trim(),
                ...(relay.password ? { password: relay.password } : {}),
              },
            }
          : { mode },
      );
      const fresh = await api.sending();
      onSaved(fresh);
      setRelay((r) => ({ ...r, password: "" }));
      refreshHealth();
      toast.success(mode === "relay" ? `Mail now leaves through ${provider?.label ?? "your relay"}.` : "This server now delivers mail directly.");
    } catch (err) {
      if (err instanceof ApiRequestError && err.field) setErrors({ [err.field]: err.message });
      else toast.error(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Page title="Sending" blurb="How mail leaves this server. Receiving always stays here, whichever you choose.">
      <form onSubmit={save} noValidate>
        <fieldset>
          <legend className="sr-only">Sending mode</legend>
          <div className="grid gap-4 md:grid-cols-2">
            <ModeCard
              value="direct"
              checked={mode === "direct"}
              onChange={() => setMode("direct")}
              title="Direct"
              summary="This server hands mail straight to each recipient's mail server."
              badge={blocked ? <StatusBadge tone="warn">Blocked here</StatusBadge> : data.mode === "direct" ? <StatusBadge tone="muted">Current</StatusBadge> : null}
              points={[
                { good: true, text: "No third party handles your outgoing mail." },
                { text: "Needs outbound port 25, reverse DNS that matches, and an IP that isn't on blocklists." },
                { text: "A new IP builds reputation slowly — start with normal, personal volumes for the first weeks." },
              ]}
              footer={blocked ? "Not possible right now: your provider blocks port 25." : undefined}
            />
            <ModeCard
              value="relay"
              checked={mode === "relay"}
              onChange={() => setMode("relay")}
              title="Relay"
              summary="A relay provider (a “smarthost”) delivers mail on this server's behalf."
              badge={blocked ? <StatusBadge tone="ok">Recommended</StatusBadge> : data.mode === "relay" ? <StatusBadge tone="muted">Current</StatusBadge> : null}
              points={[
                { good: true, text: "Works when port 25 is blocked or your IP has no reputation yet." },
                { good: true, text: "DKIM is still signed here, so DMARC stays aligned with your domains." },
                { text: "The provider sees outgoing messages, and most charge per message." },
                { text: "Its SPF include is added to each domain's record — re-publish SPF after switching." },
              ]}
            />
          </div>
        </fieldset>

        {mode === "relay" && (
          <Card className="mt-4">
            <SectionLabel>Relay connection</SectionLabel>
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-1.5 sm:col-span-2">
                <label htmlFor="rl-provider" className="block text-xs font-medium text-foreground/80">
                  Provider
                </label>
                <div className="flex flex-wrap items-center gap-3">
                  <select
                    id="rl-provider"
                    className={cn(selectClass, "max-w-xs")}
                    value={relay.provider}
                    onChange={(e) => chooseProvider(e.target.value)}
                    aria-invalid={!!errors.provider}
                    aria-describedby={errors.provider ? "rl-provider-err" : undefined}
                  >
                    {data.providers.map((p) => (
                      <option key={p.key} value={p.key}>
                        {p.label}
                      </option>
                    ))}
                  </select>
                  {provider?.docsUrl && (
                    <a
                      href={provider.docsUrl}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="inline-flex items-center gap-1 rounded-sm text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    >
                      Where to find the SMTP credentials <ExternalLink aria-hidden className="h-3 w-3" />
                      <span className="sr-only">(opens in a new tab)</span>
                    </a>
                  )}
                </div>
                <FieldError id="rl-provider-err">{errors.provider}</FieldError>
                {provider?.spfInclude && (
                  <p className="text-[11px] text-muted-foreground">
                    Adds <code className="font-mono text-foreground/80">include:{provider.spfInclude}</code> to each domain's SPF record.
                  </p>
                )}
              </div>
              <TextField
                id="rl-host"
                label="SMTP host"
                value={relay.host}
                onChange={(v) => set("host", v)}
                error={errors.host}
                placeholder={provider?.key === "ses" ? "e.g. email-smtp.eu-west-1.amazonaws.com" : "e.g. smtp.example.net"}
                hint={provider && provider.host === null ? "Your provider's regional or custom endpoint." : undefined}
                mono
              />
              <TextField id="rl-port" label="Port" value={relay.port} onChange={(v) => set("port", v.replace(/\D/g, "").slice(0, 5))} error={errors.port} inputMode="numeric" hint="TLS is required on every connection." mono />
              <TextField id="rl-user" label="Username" value={relay.username} onChange={(v) => set("username", v)} error={errors.username} autoComplete="off" />
              <TextField
                id="rl-pass"
                label="Password or API key"
                type="password"
                value={relay.password}
                onChange={(v) => set("password", v)}
                error={errors.password}
                autoComplete="new-password"
                placeholder={data.relay?.hasPassword ? "Leave blank to keep the saved one" : ""}
                hint={data.relay?.hasPassword ? "A password is saved. It's write-only — never shown again." : "Stored encrypted; never shown again after saving."}
              />
            </div>
          </Card>
        )}

        {dirty && (
          <div className="mt-4 flex flex-wrap items-center justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => {
                setMode(data.mode);
                setErrors({});
                setRelay(
                  data.relay
                    ? { provider: data.relay.provider, host: data.relay.host, port: String(data.relay.port), username: data.relay.username, password: "" }
                    : emptyRelay(data.providers[0]),
                );
              }}
            >
              Discard changes
            </Button>
            <Button type="submit" size="sm" disabled={busy}>
              {busy ? "Saving…" : mode === "direct" ? "Switch to direct sending" : data.mode === "relay" ? "Save relay settings" : "Switch to relay"}
            </Button>
          </div>
        )}
      </form>

      <Card className="mt-8">
        <SectionLabel>What this server can do</SectionLabel>
        <CheckList checks={[data.port25, data.ptr]} />
      </Card>
    </Page>
  );
}

function ModeCard({
  value,
  checked,
  onChange,
  title,
  summary,
  points,
  badge,
  footer,
}: {
  value: Mode;
  checked: boolean;
  onChange: () => void;
  title: string;
  summary: string;
  points: { good?: boolean; text: string }[];
  badge?: ReactNode;
  footer?: string;
}) {
  return (
    <label
      className={cn(
        "relative flex cursor-pointer flex-col rounded-xl border p-5 transition-colors has-[:focus-visible]:ring-1 has-[:focus-visible]:ring-ring",
        checked ? "border-foreground/35 bg-card" : "border-border/60 bg-card/40 hover:border-foreground/20",
      )}
    >
      <div className="flex items-start gap-3">
        <input type="radio" name="sending-mode" value={value} checked={checked} onChange={onChange} className="peer sr-only" />
        <span
          aria-hidden
          className={cn("mt-0.5 grid h-4 w-4 shrink-0 place-items-center rounded-full border", checked ? "border-foreground" : "border-muted-foreground/60")}
        >
          {checked && <span className="h-2 w-2 rounded-full bg-foreground" />}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-sm font-medium">{title}</span>
            {badge}
          </div>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{summary}</p>
        </div>
      </div>
      <ul className="mt-4 space-y-1.5 pl-7">
        {points.map((p) => (
          <li key={p.text} className="flex gap-1.5 text-xs leading-relaxed text-foreground/85">
            {p.good ? (
              <CheckIcon aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0 text-emerald-700 dark:text-emerald-400" />
            ) : (
              <Dot aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            )}
            <span>{p.text}</span>
          </li>
        ))}
      </ul>
      {footer && <p className="mt-4 border-t border-border/50 pl-7 pt-3 text-xs text-foreground/85">{footer}</p>}
    </label>
  );
}

function TextField({
  id,
  label,
  value,
  onChange,
  error,
  hint,
  mono,
  ...rest
}: {
  id: string;
  label: string;
  value: string;
  onChange: (v: string) => void;
  error?: string;
  hint?: string;
  mono?: boolean;
} & Omit<React.InputHTMLAttributes<HTMLInputElement>, "onChange" | "value" | "id">) {
  return (
    <div className="space-y-1.5">
      <label htmlFor={id} className="block text-xs font-medium text-foreground/80">
        {label}
      </label>
      <Input
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className={cn(mono && "font-mono text-[13px]")}
        aria-invalid={!!error}
        aria-describedby={error ? `${id}-err` : hint ? `${id}-hint` : undefined}
        spellCheck={false}
        {...rest}
      />
      {error ? (
        <FieldError id={`${id}-err`}>{error}</FieldError>
      ) : (
        hint && (
          <p id={`${id}-hint`} className="text-[11px] text-muted-foreground">
            {hint}
          </p>
        )
      )}
    </div>
  );
}
