import { useEffect, useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import { ArrowRight, Check as CheckIcon, Dot, ExternalLink, KeyRound, Route, Server, ShieldCheck } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/ui/button";
import { Card, CardBody, CardHeader } from "@/ui/card";
import { Skeleton } from "@/ui/feedback";
import { Input, RadioCard, Select, Textarea } from "@/ui/form";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { RelayProvider, SendingDomain, SendingResponse } from "@/api/types";
import { CopyButton } from "@/components/copy-button";
import { FieldError, LoadError, Page, usePageTitle } from "@/components/kit";
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
  region: string;
  spfInclude: string;
}
/** Per-domain provider records as typed/pasted (keyed by domain). */
type IdentityForm = Record<string, { dkim: string; mailFrom: string }>;

const emptyRelay = (p?: RelayProvider): RelayForm => ({
  provider: p?.key ?? "",
  host: p?.host ?? "",
  port: String(p?.port ?? 587),
  username: p?.username ?? "",
  password: "",
  region: p?.defaultRegion ?? "",
  spfInclude: "",
});

const savedRelay = (data: SendingResponse): RelayForm =>
  data.relay
    ? {
        provider: data.relay.provider,
        host: data.relay.host,
        port: String(data.relay.port),
        username: data.relay.username,
        password: "",
        region: data.relay.region ?? "",
        spfInclude: data.relay.spfInclude ?? "",
      }
    : emptyRelay(data.providers[0]);

const savedIdentities = (data: SendingResponse): IdentityForm =>
  Object.fromEntries(
    data.domains.map((d) => {
      const id = data.relay?.identities[d.name];
      return [d.name, { dkim: id?.dkimTokens?.join("\n") ?? id?.dkimValue ?? "", mailFrom: id?.mailFrom ?? "" }];
    }),
  );

const sameIdentities = (a: IdentityForm, b: IdentityForm) =>
  Object.keys({ ...a, ...b }).every((k) => (a[k]?.dkim ?? "").trim() === (b[k]?.dkim ?? "").trim() && (a[k]?.mailFrom ?? "").trim() === (b[k]?.mailFrom ?? "").trim());

const regionHost = (p: RelayProvider | undefined, region: string) => (p?.hostTemplate && region ? p.hostTemplate.replace("{region}", region) : null);

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
          <Skeleton className="h-56 rounded-xl" />
          <Skeleton className="h-56 rounded-xl" />
        </div>
        <Skeleton className="mt-4 h-40 rounded-xl" />
      </Page>
    );
  }
  return <SendingForm data={data} onSaved={setData} />;
}

function SendingForm({ data, onSaved }: { data: SendingResponse; onSaved: (d: SendingResponse) => void }) {
  const { refreshHealth } = useShell();
  const [mode, setMode] = useState<Mode>(data.mode);
  const [relay, setRelay] = useState<RelayForm>(() => savedRelay(data));
  const [ids, setIds] = useState<IdentityForm>(() => savedIdentities(data));
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<Record<string, string>>({});

  useEffect(() => setMode(data.mode), [data.mode]);

  const blocked = data.port25.status === "fail";
  const provider = data.providers.find((p) => p.key === relay.provider);
  const derivedHost = regionHost(provider, relay.region);
  const verifies = !!provider && provider.verify !== "none";
  const dirty = useMemo(() => {
    if (mode !== data.mode) return true;
    if (mode === "direct") return false;
    const r = savedRelay(data);
    return (
      r.provider !== relay.provider ||
      (!derivedHost && r.host !== relay.host) ||
      r.port !== relay.port ||
      r.username !== relay.username ||
      r.region !== relay.region ||
      r.spfInclude !== relay.spfInclude ||
      relay.password !== "" ||
      (verifies && !sameIdentities(ids, savedIdentities(data)))
    );
  }, [mode, relay, ids, data, derivedHost, verifies]);

  const set = (k: keyof RelayForm, v: string) => setRelay((r) => ({ ...r, [k]: v }));
  const setId = (domain: string, k: "dkim" | "mailFrom", v: string) =>
    setIds((m) => ({ ...m, [domain]: { dkim: m[domain]?.dkim ?? "", mailFrom: m[domain]?.mailFrom ?? "", [k]: v } }));
  const chooseProvider = (key: string) => {
    const p = data.providers.find((x) => x.key === key);
    const saved = data.relay?.provider === key ? savedRelay(data) : null;
    setErrors({});
    setRelay((r) => ({
      ...r,
      provider: key,
      host: saved?.host ?? p?.host ?? "",
      port: saved?.port ?? String(p?.port ?? 587),
      username: saved?.username ?? p?.username ?? "",
      region: saved?.region || p?.defaultRegion || "",
      spfInclude: saved?.spfInclude ?? "",
    }));
    setIds(savedIdentities(data));
  };

  async function save(e: React.FormEvent) {
    e.preventDefault();
    const errs: Record<string, string> = {};
    if (mode === "relay") {
      if (!provider) errs.provider = "Choose a relay provider.";
      if (provider?.regions.length && !relay.region) errs.region = `Choose your ${provider.label} region.`;
      if (!derivedHost && !relay.host.trim()) errs.host = "Enter the relay's SMTP host.";
      const port = Number(relay.port);
      if (!Number.isInteger(port) || port < 1 || port > 65535) errs.port = "Between 1 and 65535.";
      if (!relay.username.trim()) errs.username = "Enter the relay username.";
      const keepsPassword = data.relay?.hasPassword && data.relay.provider === relay.provider;
      if (!relay.password && !keepsPassword) errs.password = `Enter the ${provider?.secretLabel.toLowerCase() ?? "relay password"}.`;
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
                host: derivedHost ?? relay.host.trim(),
                port: Number(relay.port),
                username: relay.username.trim(),
                ...(relay.password ? { password: relay.password } : {}),
                ...(relay.region ? { region: relay.region } : {}),
                ...(provider?.key === "custom" ? { spfInclude: relay.spfInclude.trim() } : {}),
                ...(verifies ? { identities: Object.fromEntries(data.domains.map((d) => [d.name, { dkim: ids[d.name]?.dkim ?? "", mailFrom: ids[d.name]?.mailFrom ?? "" }])) } : {}),
              },
            }
          : { mode },
      );
      const fresh = await api.sending();
      onSaved(fresh);
      setRelay(savedRelay(fresh));
      setIds(savedIdentities(fresh));
      refreshHealth();
      toast.success(mode === "relay" ? `Mail now leaves through ${provider?.label ?? "your relay"}.` : "This server now delivers mail directly.");
    } catch (err) {
      if (err instanceof ApiRequestError && err.field) setErrors({ [err.field]: err.message });
      else toast.error(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  const savedHere = data.mode === "relay" && data.relay?.provider === relay.provider;
  const domainsUrl = provider?.domainsUrl?.replace("{region}", relay.region || provider.defaultRegion || "us-east-1") ?? null;

  return (
    <Page
      title="Sending"
      blurb="How mail leaves this server. Receiving always stays here, whichever you choose."
      meta={
        <StatusBadge tone={data.mode === "relay" ? "info" : blocked ? "warn" : "ok"} icon={data.mode === "relay" ? Route : Server}>
          {data.mode === "relay" ? `Relaying through ${data.providers.find((p) => p.key === data.relay?.provider)?.label ?? "a relay"}` : "Sending directly"}
        </StatusBadge>
      }
    >
      <div className="grid gap-6 xl:grid-cols-12">
      <form onSubmit={save} noValidate className="min-w-0 xl:col-span-8">
        <fieldset>
          <legend className="mb-3 text-base font-semibold tracking-[-0.01em]">How mail leaves</legend>
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
              summary="Amazon SES, Resend or another provider delivers mail on this server's behalf."
              badge={blocked ? <StatusBadge tone="ok">Recommended</StatusBadge> : data.mode === "relay" ? <StatusBadge tone="muted">Current</StatusBadge> : null}
              points={[
                { good: true, text: "Works when port 25 is blocked or your IP has no reputation yet." },
                { good: true, text: "DKIM is still signed here, so DMARC stays aligned with your domains." },
                { text: "The provider sees outgoing messages, and most charge per message." },
                { text: "The provider's DNS records join each domain's checklist, so you can see when they're live." },
              ]}
            />
          </div>
        </fieldset>

        {mode === "relay" && (
          <>
            <Card padded={false} className="mt-4" aria-labelledby="rl-title">
              <CardHeader id="rl-title" title="Relay provider" icon={Route} description="Pick the service that will deliver for you, then paste its SMTP credentials." />
              <CardBody>
              <fieldset>
                <legend className="sr-only">Relay provider</legend>
                <div className="grid grid-cols-1 gap-2 min-[420px]:grid-cols-2 lg:grid-cols-4">
                  {data.providers.map((p) => (
                    <ProviderTile key={p.key} provider={p} checked={relay.provider === p.key} onChange={() => chooseProvider(p.key)} />
                  ))}
                </div>
                <FieldError id="rl-provider-err">{errors.provider}</FieldError>
              </fieldset>

              {provider && (
                <div className="mt-5 grid gap-4 border-t border-border/70 pt-5 sm:grid-cols-2">
                  {provider.regions.length > 0 && (
                    <div className="space-y-1.5">
                      <label htmlFor="rl-region" className="block text-[13px] font-medium text-foreground/90">
                        {provider.hostTemplate ? `${provider.label} region` : "Sending region"}
                      </label>
                      <Select
                        id="rl-region"
                        className="font-mono text-[13px]"
                        value={relay.region}
                        onChange={(e) => set("region", e.target.value)}
                        aria-invalid={!!errors.region}
                        aria-describedby={errors.region ? "rl-region-err" : "rl-region-hint"}
                      >
                        {(provider.regions.includes(relay.region) || !relay.region ? provider.regions : [relay.region, ...provider.regions]).map((r) => (
                          <option key={r} value={r}>
                            {r}
                          </option>
                        ))}
                      </Select>
                      {errors.region ? (
                        <FieldError id="rl-region-err">{errors.region}</FieldError>
                      ) : (
                        <p id="rl-region-hint" className="text-xs text-muted-foreground">
                          {provider.hostTemplate ? "The region where your domains are verified in SES." : `The region shown on your ${provider.label} domain page.`}
                        </p>
                      )}
                    </div>
                  )}
                  {derivedHost ? (
                    <div className="space-y-1.5">
                      <span className="block text-[13px] font-medium text-foreground/90">SMTP host</span>
                      <p className="flex h-9 items-center truncate rounded-lg border border-dashed border-border px-3 font-mono text-[13px] text-foreground/85">{derivedHost}</p>
                      <p className="text-xs text-muted-foreground">Set by the region.</p>
                    </div>
                  ) : (
                    <TextField
                      id="rl-host"
                      label="SMTP host"
                      value={relay.host}
                      onChange={(v) => set("host", v)}
                      error={errors.host}
                      placeholder="e.g. smtp.example.net"
                      hint={provider.host ? "Change it only if your provider gives you a different endpoint." : undefined}
                      mono
                    />
                  )}
                  <TextField id="rl-port" label="Port" value={relay.port} onChange={(v) => set("port", v.replace(/\D/g, "").slice(0, 5))} error={errors.port} inputMode="numeric" hint="TLS is required on every connection." mono />
                  <TextField
                    id="rl-user"
                    label={provider.key === "ses" ? "SMTP username" : "Username"}
                    value={relay.username}
                    onChange={(v) => set("username", v)}
                    error={errors.username}
                    autoComplete="off"
                    mono={provider.key === "ses"}
                    hint={provider.username ? `${provider.label} always uses “${provider.username}”.` : undefined}
                  />
                  <TextField
                    id="rl-pass"
                    label={provider.secretLabel}
                    type="password"
                    value={relay.password}
                    onChange={(v) => set("password", v)}
                    error={errors.password}
                    autoComplete="new-password"
                    placeholder={savedHere && data.relay?.hasPassword ? "Leave blank to keep the saved one" : ""}
                    hint={
                      provider.key === "ses"
                        ? "From SES → SMTP settings → Create SMTP credentials. These are not your AWS access keys."
                        : provider.key === "resend"
                          ? "A Resend API key with sending access. Stored encrypted; never shown again."
                          : savedHere && data.relay?.hasPassword
                            ? "Saved. It's write-only — never shown again."
                            : "Stored encrypted; never shown again after saving."
                    }
                  />
                  {provider.key === "custom" && (
                    <TextField
                      id="rl-spf"
                      label="SPF include (optional)"
                      value={relay.spfInclude}
                      onChange={(v) => set("spfInclude", v)}
                      error={errors.spfInclude}
                      placeholder="e.g. spf.example.net"
                      hint="Added to each domain's SPF record, if your provider asks for one."
                      mono
                    />
                  )}
                  <div className="space-y-1 rounded-lg bg-muted/50 px-3 py-2.5 text-xs leading-relaxed text-muted-foreground sm:col-span-2">
                    {provider.spfInclude && (
                      <p>
                        Adds <code className="font-mono text-foreground/80">include:{provider.spfInclude}</code> to each domain's SPF record.
                      </p>
                    )}
                    {provider.mailFrom?.label && (
                      <p>
                        Bounces go through <code className="font-mono text-foreground/80">{provider.mailFrom.label}.&lt;domain&gt;</code>, so your main SPF record stays as it is.
                      </p>
                    )}
                    {provider.docsUrl && (
                      <a
                        href={provider.docsUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="inline-flex items-center gap-1 rounded-sm underline underline-offset-2 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                      >
                        Where to find the SMTP credentials <ExternalLink aria-hidden className="h-3 w-3" />
                        <span className="sr-only">(opens in a new tab)</span>
                      </a>
                    )}
                  </div>
                </div>
              )}
              </CardBody>
            </Card>

            {provider && verifies && (
              <Card padded={false} className="mt-4" aria-labelledby="rl-verify">
                <CardHeader
                  id="rl-verify"
                  icon={ShieldCheck}
                  title={`Verify your domains with ${provider.label}`}
                  description={
                    provider.verify === "sesDkim"
                      ? "Add each domain as an identity in Amazon SES with Easy DKIM, then paste the three CNAME records SES shows. Yunori adds them to the domain's DNS checklist and checks them against public DNS."
                      : `Add each domain in ${provider.label}, then paste the DKIM value it shows for ${provider.dkimSelector}._domainkey. Yunori adds it, and the ${provider.mailFrom?.label ?? "bounce"} records ${provider.label} needs, to the domain's DNS checklist.`
                  }
                  actions={domainsUrl && (
                    <a
                      href={domainsUrl}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="inline-flex items-center gap-1 rounded-sm text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                    >
                      Open {provider.label} domains <ExternalLink aria-hidden className="h-3 w-3" />
                      <span className="sr-only">(opens in a new tab)</span>
                    </a>
                  )}
                />
                <CardBody>
                <ul className="divide-y divide-border/60">
                  {data.domains.map((d) => (
                    <DomainVerification
                      key={d.id}
                      domain={d}
                      provider={provider}
                      saved={savedHere}
                      value={ids[d.name] ?? { dkim: "", mailFrom: "" }}
                      onChange={(k, v) => setId(d.name, k, v)}
                      errors={errors}
                    />
                  ))}
                </ul>
                {data.domains.length === 0 && <p className="text-[13px] text-muted-foreground">Add a domain first.</p>}
                </CardBody>
              </Card>
            )}
          </>
        )}

        {dirty && (
          <div className="sticky bottom-3 z-10 mt-4 flex flex-wrap items-center justify-end gap-2 rounded-xl border border-border bg-popover/95 p-3 shadow-pop backdrop-blur">
            <span className="mr-auto pl-1 text-[13px] text-muted-foreground">You have unsaved changes.</span>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => {
                setMode(data.mode);
                setErrors({});
                setRelay(savedRelay(data));
                setIds(savedIdentities(data));
              }}
            >
              Discard changes
            </Button>
            <Button type="submit" size="sm" loading={busy}>
              {busy ? "Saving…" : mode === "direct" ? "Switch to direct sending" : data.mode === "relay" ? "Save relay settings" : "Switch to relay"}
            </Button>
          </div>
        )}
      </form>

      <aside className="min-w-0 space-y-4 xl:col-span-4">
        <Card padded={false} aria-labelledby="can-title">
          <CardHeader id="can-title" title="What this server can do" icon={Server} description="Direct sending needs both of these to pass." divider />
          <CardBody className="pb-2">
            <CheckList checks={[data.port25, data.ptr]} compact />
          </CardBody>
        </Card>
        <Card padded={false} aria-labelledby="keys-title">
          <CardHeader id="keys-title" title="Either way" icon={KeyRound} />
          <CardBody>
            <ul className="space-y-2.5 text-[13px] leading-relaxed text-muted-foreground">
              <li className="flex gap-2">
                <CheckIcon aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-success-ink" />
                DKIM is signed on this server with your domain's own keys.
              </li>
              <li className="flex gap-2">
                <CheckIcon aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-success-ink" />
                Relay passwords are sealed in the vault and never shown again.
              </li>
              <li className="flex gap-2">
                <CheckIcon aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-success-ink" />
                Incoming mail always arrives here directly.
              </li>
            </ul>
          </CardBody>
        </Card>
      </aside>
      </div>
    </Page>
  );
}

function ProviderTile({ provider, checked, onChange }: { provider: RelayProvider; checked: boolean; onChange: () => void }) {
  return (
    <RadioCard
      compact
      name="relay-provider"
      value={provider.key}
      checked={checked}
      onChange={onChange}
      title={provider.label}
      description={provider.verify !== "none" ? "Domain records set up here" : provider.key === "custom" ? "Any SMTP relay" : "SMTP relay"}
    />
  );
}

function DomainVerification({
  domain,
  provider,
  saved,
  value,
  onChange,
  errors,
}: {
  domain: SendingDomain;
  provider: RelayProvider;
  /** The page's saved provider is this one, so `domain.records` describe it. */
  saved: boolean;
  value: { dkim: string; mailFrom: string };
  onChange: (k: "dkim" | "mailFrom", v: string) => void;
  errors: Record<string, string>;
}) {
  const base = `id-${domain.id}`;
  const dkimErr = errors[`identities.${domain.name}.dkim`];
  const mfErr = errors[`identities.${domain.name}.mailFrom`];
  const ses = provider.verify === "sesDkim";
  return (
    <li className="py-4 first:pt-0 last:pb-0">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-sm font-medium">{domain.name}</span>
        {saved && (domain.needsVerification ? <StatusBadge tone="warn">Needs {provider.label} records</StatusBadge> : <StatusBadge tone="ok">Records added</StatusBadge>)}
      </div>
      <div className={cn("mt-3 grid gap-3", ses && "sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]")}>
        <div className="space-y-1.5">
          <label htmlFor={`${base}-dkim`} className="block text-[13px] font-medium text-foreground/90">
            {ses ? "DKIM records from SES" : `DKIM value for ${provider.dkimSelector}._domainkey`}
          </label>
          <Textarea
            id={`${base}-dkim`}
            value={value.dkim}
            onChange={(e) => onChange("dkim", e.target.value)}
            rows={ses ? 3 : 2}
            spellCheck={false}
            className="min-h-0 font-mono text-[12px]"
            placeholder={ses ? "Paste the three CNAME rows (or just the tokens)" : "p=MIGfMA0GCSqGSIb3DQEB…"}
            aria-invalid={!!dkimErr}
            aria-describedby={dkimErr ? `${base}-dkim-err` : undefined}
          />
          <FieldError id={`${base}-dkim-err`}>{dkimErr}</FieldError>
        </div>
        {ses && (
          <div className="space-y-1.5">
            <label htmlFor={`${base}-mf`} className="block text-[13px] font-medium text-foreground/90">
              Custom MAIL FROM (optional)
            </label>
            <Input
              id={`${base}-mf`}
              value={value.mailFrom}
              onChange={(e) => onChange("mailFrom", e.target.value)}
              className="font-mono text-[13px]"
              spellCheck={false}
              placeholder={`bounce.${domain.name}`}
              aria-invalid={!!mfErr}
              aria-describedby={mfErr ? `${base}-mf-err` : `${base}-mf-hint`}
            />
            {mfErr ? (
              <FieldError id={`${base}-mf-err`}>{mfErr}</FieldError>
            ) : (
              <p id={`${base}-mf-hint`} className="text-xs text-muted-foreground">
                Set the same subdomain in SES so SPF aligns too.
              </p>
            )}
          </div>
        )}
      </div>
      {saved && domain.records.length > 0 && (
        <div className="mt-3 overflow-x-auto rounded-lg border border-border">
          <table className="w-full min-w-[32rem] table-fixed text-left text-xs">
            <caption className="sr-only">
              {provider.label} records for {domain.name}
            </caption>
            <thead className="bg-muted/40 text-xs text-muted-foreground">
              <tr>
                <th scope="col" className="w-16 px-3 py-2 font-normal">
                  Type
                </th>
                <th scope="col" className="px-3 py-2 font-normal">
                  Name
                </th>
                <th scope="col" className="px-3 py-2 font-normal">
                  Value
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border/50">
              {domain.records.map((r) => (
                <tr key={r.id}>
                  <td className="px-3 py-2 align-top font-mono">{r.type}</td>
                  <td className="px-3 py-2 align-top">
                    <span className="flex items-start gap-1.5">
                      <span className="min-w-0 break-all font-mono">{r.name}</span>
                      <CopyButton value={r.name} label={`${r.type} name`} />
                    </span>
                  </td>
                  <td className="px-3 py-2 align-top">
                    <span className="flex items-start gap-1.5">
                      <span className="min-w-0 break-all font-mono">{r.value}</span>
                      <CopyButton value={r.value} label={`${r.type} value`} />
                    </span>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {saved && domain.records.length > 0 && (
        <Link
          to={`/console/domains/${domain.id}`}
          className="mt-2 inline-flex items-center gap-1 rounded-sm text-xs text-muted-foreground underline underline-offset-2 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
        >
          See whether they're live on the DNS page <ArrowRight aria-hidden className="h-3 w-3" />
        </Link>
      )}
    </li>
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
    <RadioCard name="sending-mode" value={value} checked={checked} onChange={onChange} title={title} description={summary} badge={badge} className="h-full">
      <ul className="mt-4 space-y-2 pl-7">
        {points.map((p) => (
          <li key={p.text} className="flex gap-2 text-[13px] leading-relaxed text-foreground/85">
            {p.good ? (
              <CheckIcon aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-success-ink" />
            ) : (
              <Dot aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
            )}
            <span>{p.text}</span>
          </li>
        ))}
      </ul>
      {footer && <p className="mt-4 rounded-lg bg-warning/10 px-3 py-2 text-[13px] text-foreground/90 sm:ml-7">{footer}</p>}
    </RadioCard>
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
      <label htmlFor={id} className="block text-[13px] font-medium text-foreground/90">
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
          <p id={`${id}-hint`} className="text-xs leading-relaxed text-muted-foreground">
            {hint}
          </p>
        )
      )}
    </div>
  );
}
