import { useEffect, useState, type ReactNode } from "react";
import { Button } from "@/ui/button";
import { Input } from "@/ui/form";
import { Checkbox, Switch } from "@/ui/form";
import { Textarea } from "@/ui/form";
import { Skeleton } from "@/ui/feedback";
import { ApiRequestError, errorMessage, portalApi } from "@/api/client";
import type { MailRules } from "@/api/types";
import { FormError } from "@/components/auth-frame";
import { FieldError, LoadError } from "@/components/kit";
import { toast } from "@/lib/toast";
import { useApi } from "@/lib/use-api";

const MAX_SUBJECT = 200;
const MAX_BODY = 5000;
const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

/** Server field name → the input that shows its message. */
const INPUT_ID: Record<string, string> = {
  "vacation.subject": "vr-subject",
  "vacation.body": "vr-body",
  "vacation.startDate": "vr-start",
  "vacation.endDate": "vr-end",
  "forwarding.address": "fw-address",
};

interface Form {
  vEnabled: boolean;
  subject: string;
  body: string;
  start: string;
  end: string;
  fEnabled: boolean;
  address: string;
  keepCopy: boolean;
}

function toForm(r: MailRules): Form {
  return {
    vEnabled: r.vacation.enabled,
    subject: r.vacation.subject,
    body: r.vacation.body,
    start: r.vacation.startDate ?? "",
    end: r.vacation.endDate ?? "",
    fEnabled: r.forwarding.enabled,
    address: r.forwarding.address,
    keepCopy: r.forwarding.keepCopy,
  };
}

function toRules(f: Form): MailRules {
  return {
    vacation: {
      enabled: f.vEnabled,
      subject: f.subject,
      body: f.body,
      ...(f.start ? { startDate: f.start } : {}),
      ...(f.end ? { endDate: f.end } : {}),
    },
    forwarding: { enabled: f.fEnabled, address: f.address.trim(), keepCopy: f.keepCopy },
  };
}

/** "31 Dec" (with the year when it isn't this year) for a YYYY-MM-DD day. */
function fmtDay(d: string): string {
  const [y, m, day] = d.split("-").map(Number);
  const date = new Date(y, m - 1, day);
  if (Number.isNaN(date.getTime())) return d;
  const sameYear = y === new Date().getFullYear();
  return date.toLocaleDateString(undefined, { day: "numeric", month: "short", ...(sameYear ? {} : { year: "numeric" }) });
}

function today(): string {
  const n = new Date();
  return `${n.getFullYear()}-${String(n.getMonth() + 1).padStart(2, "0")}-${String(n.getDate()).padStart(2, "0")}`;
}

/** What is switched on right now, in a sentence each — from the saved rules, not the draft. */
export function rulesStatus(r: MailRules): string[] {
  const out: string[] = [];
  const v = r.vacation;
  if (v.enabled) {
    const t = today();
    if (v.endDate && v.endDate < t) out.push(`Auto-reply ended on ${fmtDay(v.endDate)}`);
    else if (v.startDate && v.startDate > t) out.push(`Auto-reply starts ${fmtDay(v.startDate)}${v.endDate ? `, until ${fmtDay(v.endDate)}` : ""}`);
    else out.push(v.endDate ? `Auto-reply on until ${fmtDay(v.endDate)}` : "Auto-reply on");
  }
  const f = r.forwarding;
  if (f.enabled) out.push(`Forwarding to ${f.address}${f.keepCopy ? ", keeping a copy here" : " — no copy kept here"}`);
  return out;
}

/**
 * Auto-reply (out of office) and forwarding. The server keeps them as a
 * Sieve script on the mailbox, so they work with every app closed.
 */
export function MailRulesCard({ address }: { address: string }) {
  const { data, error, reload, setData } = useApi("portal:rules", portalApi.rules);

  if (error && !data) return <LoadError error={error} onRetry={() => void reload()} what="Auto-reply and forwarding" />;
  if (!data)
    return (
      <div aria-busy="true" className="rounded-xl border border-border bg-card shadow-card p-5">
        <Skeleton className="h-5 w-48" />
        <Skeleton className="mt-6 h-5 w-56" />
      </div>
    );
  return <RulesForm saved={data} address={address} onSaved={setData} />;
}

function RulesForm({ saved, address, onSaved }: { saved: MailRules; address: string; onSaved: (r: MailRules) => void }) {
  const [form, setForm] = useState<Form>(() => toForm(saved));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [focusField, setFocusField] = useState<string | null>(null);
  const dirty = JSON.stringify(toRules(form)) !== JSON.stringify(toRules(toForm(saved)));
  const status = rulesStatus(saved);

  useEffect(() => {
    if (!focusField) return;
    document.getElementById(INPUT_ID[focusField] ?? "")?.focus();
    setFocusField(null);
  }, [focusField]);

  const set = <K extends keyof Form>(k: K, v: Form[K]) => {
    setForm((f) => ({ ...f, [k]: v }));
    const field = { subject: "vacation.subject", body: "vacation.body", start: "vacation.startDate", end: "vacation.endDate", address: "forwarding.address" }[k as string];
    if (field && errors[field]) setErrors(({ [field]: _gone, ...rest }) => rest);
  };

  async function save(e: React.FormEvent) {
    e.preventDefault();
    const errs: Record<string, string> = {};
    if (form.vEnabled && !form.body.trim()) errs["vacation.body"] = "Write the reply people will get.";
    if (form.vEnabled && form.start && form.end && form.end < form.start) errs["vacation.endDate"] = "The last day comes before the first day.";
    if (form.fEnabled && !EMAIL_RE.test(form.address.trim())) errs["forwarding.address"] = "Enter a valid email address.";
    setErrors(errs);
    const first = Object.keys(INPUT_ID).find((k) => errs[k]);
    if (first) {
      setFocusField(first);
      return;
    }
    setBusy(true);
    try {
      const r = await portalApi.saveRules(toRules(form));
      onSaved(r);
      setForm(toForm(r));
      const now = rulesStatus(r);
      toast.success(now.length ? `Saved. ${now.join(". ")}.` : "Saved. Auto-reply and forwarding are off.");
    } catch (err) {
      if (err instanceof ApiRequestError && err.field && INPUT_ID[err.field]) {
        setErrors({ [err.field]: err.message });
        setFocusField(err.field);
      } else setErrors({ form: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={save} noValidate className="rounded-xl border border-border bg-card shadow-card">
      <Toggle
        id="vr"
        title="Auto-reply"
        hint="Answers each sender at most once a day while you're away."
        checked={form.vEnabled}
        onChange={(v) => set("vEnabled", v)}
      >
        <div className="space-y-1.5">
          <FieldLabel htmlFor="vr-subject">Subject</FieldLabel>
          <Input
            id="vr-subject"
            value={form.subject}
            onChange={(e) => set("subject", e.target.value)}
            placeholder="Out of office"
            maxLength={MAX_SUBJECT}
            aria-invalid={!!errors["vacation.subject"]}
            aria-describedby={errors["vacation.subject"] ? "vr-subject-err" : undefined}
          />
          <FieldError id="vr-subject-err">{errors["vacation.subject"]}</FieldError>
        </div>
        <div className="space-y-1.5">
          <FieldLabel htmlFor="vr-body">Message</FieldLabel>
          <Textarea
            id="vr-body"
            value={form.body}
            onChange={(e) => set("body", e.target.value)}
            rows={5}
            maxLength={MAX_BODY}
            placeholder="I'm away until Monday 5 January and reading mail only now and then. For anything urgent, write to …"
            aria-invalid={!!errors["vacation.body"]}
            aria-describedby={errors["vacation.body"] ? "vr-body-err" : undefined}
          />
          {errors["vacation.body"] ? (
            <FieldError id="vr-body-err">{errors["vacation.body"]}</FieldError>
          ) : (
            form.body.length > MAX_BODY - 500 && (
              <p className="text-xs tabular-nums text-muted-foreground">
                {form.body.length} of {MAX_BODY} characters
              </p>
            )
          )}
        </div>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-1.5">
            <FieldLabel htmlFor="vr-start" optional>
              From
            </FieldLabel>
            <Input
              id="vr-start"
              type="date"
              className="dark:[color-scheme:dark]"
              value={form.start}
              onChange={(e) => set("start", e.target.value)}
              aria-invalid={!!errors["vacation.startDate"]}
              aria-describedby={errors["vacation.startDate"] ? "vr-start-err" : "vr-start-hint"}
            />
            {errors["vacation.startDate"] ? (
              <FieldError id="vr-start-err">{errors["vacation.startDate"]}</FieldError>
            ) : (
              <p id="vr-start-hint" className="text-xs text-muted-foreground">
                Empty: starts as soon as you save.
              </p>
            )}
          </div>
          <div className="space-y-1.5">
            <FieldLabel htmlFor="vr-end" optional>
              Until
            </FieldLabel>
            <Input
              id="vr-end"
              type="date"
              className="dark:[color-scheme:dark]"
              value={form.end}
              min={form.start || undefined}
              onChange={(e) => set("end", e.target.value)}
              aria-invalid={!!errors["vacation.endDate"]}
              aria-describedby={errors["vacation.endDate"] ? "vr-end-err" : "vr-end-hint"}
            />
            {errors["vacation.endDate"] ? (
              <FieldError id="vr-end-err">{errors["vacation.endDate"]}</FieldError>
            ) : (
              <p id="vr-end-hint" className="text-xs text-muted-foreground">
                Empty: until you switch it off. Includes that day.
              </p>
            )}
          </div>
        </div>
      </Toggle>

      <Toggle
        id="fw"
        title="Forwarding"
        hint="Sends every new message on to another address."
        checked={form.fEnabled}
        onChange={(v) => set("fEnabled", v)}
        bordered
      >
        <div className="space-y-1.5">
          <FieldLabel htmlFor="fw-address">Forward to</FieldLabel>
          <Input
            id="fw-address"
            type="email"
            inputMode="email"
            autoComplete="email"
            autoCapitalize="none"
            spellCheck={false}
            value={form.address}
            onChange={(e) => set("address", e.target.value)}
            placeholder="name@example.org"
            aria-invalid={!!errors["forwarding.address"]}
            aria-describedby={errors["forwarding.address"] ? "fw-address-err" : undefined}
          />
          <FieldError id="fw-address-err">{errors["forwarding.address"]}</FieldError>
        </div>
        <label className="flex cursor-pointer items-start gap-2.5 text-sm">
          <Checkbox checked={form.keepCopy} onChange={(e) => set("keepCopy", e.target.checked)} className="mt-0.5" />
          <span>
            <span className="text-foreground/90">Keep a copy here</span>
            <span className="block text-xs text-muted-foreground">
              {form.keepCopy
                ? `Mail still arrives in ${address} as well.`
                : `Nothing stays in ${address} — if the other address refuses a message, it is lost.`}
            </span>
          </span>
        </label>
      </Toggle>

      <div className="flex flex-col gap-3 border-t border-border/70 px-5 py-4 sm:flex-row sm:items-center">
        <p role="status" className="min-w-0 flex-1 text-xs leading-relaxed text-muted-foreground">
          {status.length ? (
            status.map((s) => (
              <span key={s} className="block text-foreground/85">
                <span aria-hidden className="mr-1.5 inline-block h-1.5 w-1.5 -translate-y-px rounded-full bg-success" />
                {s}
              </span>
            ))
          ) : (
            "Both are off."
          )}
          {dirty && <span className="mt-0.5 block text-warning-ink">You have changes that aren't saved yet.</span>}
        </p>
        {errors.form && <FormError>{errors.form}</FormError>}
        <Button type="submit" size="sm" disabled={!dirty} loading={busy} className="shrink-0 self-end sm:self-auto">
          {busy ? "Saving…" : "Save"}
        </Button>
      </div>
    </form>
  );
}

function Toggle({
  id,
  title,
  hint,
  checked,
  onChange,
  bordered,
  children,
}: {
  id: string;
  title: string;
  hint: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  bordered?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={bordered ? "border-t border-border/70" : undefined}>
      <div className="flex items-start gap-4 px-4 py-4 sm:px-5">
        <div className="min-w-0 flex-1">
          <label htmlFor={`${id}-toggle`} className="cursor-pointer text-sm font-medium">
            {title}
          </label>
          <p id={`${id}-hint`} className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">
            {hint}
          </p>
        </div>
        <Switch id={`${id}-toggle`} checked={checked} onCheckedChange={onChange} aria-describedby={`${id}-hint`} className="mt-0.5" />
      </div>
      {checked && <div className="space-y-4 px-4 pb-5 sm:px-5">{children}</div>}
    </div>
  );
}

function FieldLabel({ htmlFor, optional, children }: { htmlFor: string; optional?: boolean; children: ReactNode }) {
  return (
    <label htmlFor={htmlFor} className="block text-[13px] font-medium text-foreground/90">
      {children}
      {optional && <span className="font-normal text-muted-foreground"> (optional)</span>}
    </label>
  );
}
