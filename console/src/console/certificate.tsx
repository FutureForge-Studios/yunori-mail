import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronRight, RefreshCw, ShieldCheck } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { Button } from "@/design/vendor/ui/button";
import { Chip, KV, SectionLabel } from "@/design/vendor/admin/ui";
import { api, errorMessage } from "@/api/client";
import type { CertificateInfo, CertState } from "@/api/types";
import { StatusBadge, type Tone } from "@/components/status";
import { fmtDate, relativeDays, timeAgo } from "@/lib/format";
import { toast } from "@/lib/toast";

export const CERT_STATE: Record<CertState, { label: string; tone: Tone }> = {
  valid: { label: "Valid", tone: "ok" },
  expiring: { label: "Expiring soon", tone: "warn" },
  expired: { label: "Expired", tone: "bad" },
  self_signed: { label: "Temporary", tone: "warn" },
  test: { label: "Test certificate", tone: "warn" },
  missing: { label: "Not issued yet", tone: "muted" },
};

/** "in 4 minutes" / "now" for the next automatic attempt. */
export function untilText(iso: string | null): string {
  if (!iso) return "soon";
  const s = Math.round((new Date(iso).getTime() - Date.now()) / 1000);
  if (s <= 5) return "any moment now";
  if (s < 90) return `in ${s} seconds`;
  if (s < 90 * 60) return `in ${Math.round(s / 60)} minutes`;
  const h = Math.round(s / 3600);
  return `in about ${h} hour${h === 1 ? "" : "s"}`;
}

/**
 * Automatic-HTTPS status, polled every 3 s while an attempt runs (and every
 * 20 s otherwise when `live`). `tryNow` asks for an attempt immediately.
 */
export function useCertificate({ live = false, onIssued }: { live?: boolean; onIssued?: () => void } = {}) {
  const [cert, setCert] = useState<CertificateInfo | null>(null);
  const [error, setError] = useState<unknown>(null);
  const [requested, setRequested] = useState(false);
  const wasMissing = useRef<boolean | null>(null);
  // Held in a ref so an inline callback doesn't restart loading on every render.
  const onIssuedRef = useRef(onIssued);
  useEffect(() => {
    onIssuedRef.current = onIssued;
  });

  const load = useCallback(async () => {
    try {
      const c = await api.certificate();
      setCert(c);
      setError(null);
      const usable = c.state === "valid" || c.state === "expiring";
      if (wasMissing.current === true && usable) onIssuedRef.current?.();
      wasMissing.current = !usable;
      return c;
    } catch (err) {
      setError(err);
      return null;
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const busy = requested || !!cert?.inProgress;
  useEffect(() => {
    if (!busy && !live) return;
    const t = window.setInterval(
      () => {
        if (document.visibilityState !== "visible") return;
        void load().then((c) => {
          if (c && !c.inProgress) setRequested(false);
        });
      },
      busy ? 3000 : 20000,
    );
    return () => window.clearInterval(t);
  }, [busy, live, load]);

  const tryNow = useCallback(async () => {
    setRequested(true);
    try {
      const c = await api.renewCertificate();
      setCert(c);
      if (!c.inProgress) setRequested(false);
    } catch (err) {
      setRequested(false);
      toast.error(errorMessage(err));
    }
  }, []);

  return { cert, error, busy, tryNow, reload: load };
}

/** What went wrong with the last attempt, what to do, and "Try now". */
export function CertificateProblem({
  cert,
  busy,
  onTryNow,
  fallback,
}: {
  cert: CertificateInfo | null;
  busy: boolean;
  onTryNow: () => void;
  fallback?: string | null;
}) {
  const p = cert?.problem ?? null;
  return (
    <div className="space-y-2.5">
      {p ? (
        <>
          <p className="text-sm leading-relaxed text-foreground">{p.summary}</p>
          <p className="text-sm leading-relaxed text-foreground/85">{p.remedy}</p>
        </>
      ) : (
        <p className="text-sm leading-relaxed text-foreground">{fallback ?? "The certificate has not been issued yet."}</p>
      )}
      {cert?.state === "self_signed" && (
        <p className="text-xs leading-relaxed text-muted-foreground">
          Until then the console uses a temporary self-signed certificate, so browsers warn about this page. Mail apps should wait for the real one.
        </p>
      )}
      {p?.detail && (
        <details className="group">
          <summary className="flex cursor-pointer list-none items-center gap-1 rounded-sm text-[11px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
            <ChevronRight aria-hidden className="h-3 w-3 transition-transform group-open:rotate-90 motion-reduce:transition-none" />
            What {cert?.authority ?? "the certificate authority"} said
          </summary>
          <p className="mt-1.5 break-words rounded-md border border-border/60 bg-card px-2.5 py-2 font-mono text-[11px] leading-relaxed text-foreground/80">{p.detail}</p>
        </details>
      )}
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 pt-0.5">
        <Button size="sm" onClick={onTryNow} disabled={busy}>
          <RefreshCw aria-hidden className={cn("h-3.5 w-3.5", busy && "animate-spin motion-reduce:animate-none")} />
          {busy ? "Requesting…" : "Try now"}
        </Button>
        <span className="text-[11px] text-muted-foreground" aria-live="polite">
          {busy
            ? `Asking ${cert?.authority ?? "the certificate authority"} — this takes up to a minute.`
            : cert?.nextAttempt
              ? `Yunori tries again by itself ${untilText(cert.nextAttempt)}${cert.lastAttempt ? ` · last attempt ${timeAgo(cert.lastAttempt)}` : ""}.`
              : "Yunori keeps trying by itself."}
        </span>
      </div>
    </div>
  );
}

/** The System page's certificate panel. */
export function CertificatePanel() {
  const { cert, busy, tryNow } = useCertificate({ live: true });
  if (!cert) return <p className="text-xs text-muted-foreground">Loading…</p>;
  const meta = CERT_STATE[cert.state];
  const healthy = cert.state === "valid";
  return (
    <div>
      <div className="mb-1 flex items-center justify-between gap-2">
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <ShieldCheck aria-hidden className="h-3.5 w-3.5" />
          {cert.authority} · automatic
        </span>
        <StatusBadge tone={meta.tone}>{meta.label}</StatusBadge>
      </div>
      {cert.state !== "missing" && (
        <>
          <KV label="Expires">{cert.notAfter ? `${fmtDate(cert.notAfter)} (${relativeDays(cert.notAfter)})` : "—"}</KV>
          {healthy && cert.renewsAt && <KV label="Renews">{new Date(cert.renewsAt).getTime() > Date.now() ? `from ${fmtDate(cert.renewsAt)}` : "now"}</KV>}
          <div className="flex items-center justify-between gap-4 py-1.5">
            <span className="text-xs text-muted-foreground">Served on 465/993</span>
            {cert.servedMatches === null ? (
              <StatusBadge tone="muted">Not checked</StatusBadge>
            ) : cert.servedMatches ? (
              <StatusBadge tone="ok">Matches</StatusBadge>
            ) : (
              <StatusBadge tone="warn">Different certificate</StatusBadge>
            )}
          </div>
          <div className="mt-2 flex flex-wrap gap-1">
            {cert.hostnames.map((h) => (
              <Chip key={h}>{h}</Chip>
            ))}
          </div>
        </>
      )}
      {!healthy && (
        <div className="mt-3 rounded-lg border border-border bg-muted/30 p-3">
          <SectionLabel>{cert.problem ? "Last attempt" : "Next step"}</SectionLabel>
          <CertificateProblem
            cert={cert}
            busy={busy}
            onTryNow={() => void tryNow()}
            fallback={cert.state === "missing" ? "It is issued once the mail hostname points at this server." : null}
          />
        </div>
      )}
    </div>
  );
}
