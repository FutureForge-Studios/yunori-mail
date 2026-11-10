import { useEffect, useRef, useState } from "react";
import { AlertCircle, CheckCircle2, Download, FileUp } from "lucide-react";
import { Button } from "@/design/vendor/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/design/vendor/ui/dialog";
import { Textarea } from "@/design/vendor/ui/textarea";
import { ApiRequestError, api, errorMessage } from "@/api/client";
import type { ImportMailboxesResponse, ImportSkipped } from "@/api/types";
import { CopyButton } from "@/components/copy-button";
import { FieldError, RadioRow } from "@/components/kit";
import { saveBlob, toCsv } from "@/lib/download";
import { fmtDate, fmtNumber } from "@/lib/format";
import { toast } from "@/lib/toast";

/** The server reads at most this many rows; a file far bigger than that is surely the wrong one. */
const MAX_FILE_BYTES = 2 * 1024 * 1024;

const EXAMPLE = "address,display_name,quota_mb\nren@example.com,Ren Ishikawa,5120\nmina@example.com,Mina Sato,";

type Mode = "invite" | "password";
type Step =
  | { kind: "input" }
  | { kind: "preview"; result: ImportMailboxesResponse }
  | { kind: "done"; result: ImportMailboxesResponse; mode: Mode };

const plural = (n: number, one: string, many = `${one}s`) => `${fmtNumber(n)} ${n === 1 ? one : many}`;

/**
 * Import mailboxes from CSV: check first (a dry run that creates nothing),
 * then import. Invite links or generated passwords are shown once, with a
 * CSV download of the results.
 */
export function ImportMailboxesDialog({
  open,
  onOpenChange,
  onImported,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onImported: () => void;
}) {
  const [csv, setCsv] = useState("");
  const [fileName, setFileName] = useState<string | null>(null);
  const [mode, setMode] = useState<Mode>("invite");
  const [step, setStep] = useState<Step>({ kind: "input" });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ csv?: string; form?: string }>({});
  const fileRef = useRef<HTMLInputElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    if (step.kind !== "input") headingRef.current?.focus();
  }, [step.kind]);

  const reset = () => {
    setCsv("");
    setFileName(null);
    setMode("invite");
    setStep({ kind: "input" });
    setError({});
  };

  function close() {
    if (busy) return;
    onOpenChange(false);
    reset();
  }

  async function pick(file: File | undefined) {
    if (!file) return;
    if (file.size > MAX_FILE_BYTES) {
      setError({ csv: "That file is too big for an import. Use a CSV with at most 1,000 mailboxes." });
      return;
    }
    try {
      setCsv(await file.text());
      setFileName(file.name);
      setError({});
    } catch {
      setError({ csv: "That file couldn't be read. Save it as CSV (UTF-8) and try again." });
    } finally {
      if (fileRef.current) fileRef.current.value = "";
    }
  }

  async function run(dryRun: boolean) {
    if (!csv.trim()) {
      setError({ csv: "Choose a CSV file, or paste the rows." });
      return;
    }
    setBusy(true);
    setError({});
    try {
      const result = await api.importMailboxes({ csv, invite: mode === "invite", dryRun });
      if (dryRun) setStep({ kind: "preview", result });
      else {
        setStep({ kind: "done", result, mode });
        if (result.created.length) onImported();
        toast.success(result.created.length ? `${plural(result.created.length, "mailbox", "mailboxes")} created.` : "No mailboxes were created.");
      }
    } catch (err) {
      setStep({ kind: "input" });
      if (err instanceof ApiRequestError && err.field === "csv") setError({ csv: sentence(err.message) });
      else setError({ form: errorMessage(err) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent
        className="max-h-[calc(100dvh-2rem)] max-w-2xl overflow-y-auto"
        // The results are shown once: don't lose them to a stray click outside.
        onInteractOutside={(e) => step.kind === "done" && e.preventDefault()}
      >
        {step.kind === "input" && (
          <form
            className="space-y-4"
            noValidate
            onSubmit={(e) => {
              e.preventDefault();
              void run(true);
            }}
          >
            <DialogHeader>
              <DialogTitle>Import mailboxes</DialogTitle>
              <DialogDescription>
                One mailbox per line: the address, then optionally a display name and a storage limit in MB. A header row is fine, and so
                is a file exported from this page. Nothing is created until you've checked the list.
              </DialogDescription>
            </DialogHeader>

            <div className="space-y-1.5">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <label htmlFor="imp-csv" className="block text-xs font-medium text-foreground/80">
                  CSV
                </label>
                <div className="flex items-center gap-2">
                  {fileName && <span className="max-w-48 truncate text-[11px] text-muted-foreground">{fileName}</span>}
                  <input
                    ref={fileRef}
                    type="file"
                    accept=".csv,text/csv,text/plain"
                    className="sr-only"
                    tabIndex={-1}
                    aria-hidden
                    onChange={(e) => void pick(e.target.files?.[0])}
                  />
                  <Button type="button" variant="outline" size="sm" onClick={() => fileRef.current?.click()}>
                    <FileUp className="h-3.5 w-3.5" />
                    {fileName ? "Choose another file" : "Choose a .csv file"}
                  </Button>
                </div>
              </div>
              <Textarea
                id="imp-csv"
                value={csv}
                onChange={(e) => {
                  setCsv(e.target.value);
                  setFileName(null);
                  if (error.csv) setError({});
                }}
                rows={7}
                spellCheck={false}
                placeholder={`…or paste the rows here:\n\n${EXAMPLE}`}
                className="font-mono text-xs"
                aria-invalid={!!error.csv}
                aria-describedby={error.csv ? "imp-csv-err" : "imp-csv-hint"}
              />
              {error.csv ? (
                <FieldError id="imp-csv-err">{error.csv}</FieldError>
              ) : (
                <p id="imp-csv-hint" className="text-[11px] text-muted-foreground">
                  Up to 1,000 mailboxes at a time. Their domains must already be on this server. An empty storage limit means no limit.
                </p>
              )}
            </div>

            <fieldset>
              <legend className="mb-1.5 text-xs font-medium text-foreground/80">How they'll get in</legend>
              <div className="space-y-1.5">
                <RadioRow
                  name="imp-mode"
                  checked={mode === "invite"}
                  onChange={() => setMode("invite")}
                  label="Send invite links"
                  hint="Recommended. Each person chooses their own password. You get one link per mailbox to pass on; each works once, for 7 days."
                />
                <RadioRow
                  name="imp-mode"
                  checked={mode === "password"}
                  onChange={() => setMode("password")}
                  label="Generate passwords"
                  hint="A strong password per mailbox, shown once after the import for you to pass on."
                />
              </div>
            </fieldset>

            {error.form && (
              <p role="alert" className="rounded-md border border-border bg-muted/40 px-3 py-2 text-xs">
                {error.form}
              </p>
            )}

            <DialogFooter>
              <Button type="button" variant="ghost" onClick={close} disabled={busy}>
                Cancel
              </Button>
              <Button type="submit" disabled={busy || !csv.trim()}>
                {busy ? "Checking…" : "Check"}
              </Button>
            </DialogFooter>
          </form>
        )}

        {step.kind === "preview" && (
          <div className="space-y-4">
            <DialogHeader>
              <DialogTitle ref={headingRef} tabIndex={-1} className="focus:outline-none">
                {step.result.ready.length ? `${plural(step.result.ready.length, "mailbox", "mailboxes")} ready to import` : "Nothing to import"}
              </DialogTitle>
              <DialogDescription>
                {step.result.ready.length
                  ? `Checked and not created yet. ${mode === "invite" ? "Each gets an invite link" : "Each gets a generated password"}, shown after the import.`
                  : "None of the rows can be created. Fix the file and check again."}
                {step.result.skipped.length > 0 && ` ${plural(step.result.skipped.length, "row")} will be skipped.`}
              </DialogDescription>
            </DialogHeader>

            {step.result.ready.length > 0 && (
              <div>
                <p className="mb-1.5 flex items-center gap-1.5 text-xs font-medium text-foreground/80">
                  <CheckCircle2 aria-hidden className="h-3.5 w-3.5 text-emerald-700 dark:text-emerald-400" />
                  Ready
                </p>
                <ul className="max-h-48 overflow-y-auto rounded-lg border border-border/60 bg-muted/30 px-3 py-2 font-mono text-xs leading-6 text-foreground/90">
                  {step.result.ready.map((a) => (
                    <li key={a} className="truncate">
                      {a}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            <SkippedList rows={step.result.skipped} />

            <DialogFooter className="flex-wrap">
              <Button type="button" variant="ghost" onClick={() => setStep({ kind: "input" })} disabled={busy}>
                Back
              </Button>
              <Button type="button" onClick={() => void run(false)} disabled={busy || step.result.ready.length === 0}>
                {busy ? "Importing…" : `Import ${plural(step.result.ready.length, "mailbox", "mailboxes")}`}
              </Button>
            </DialogFooter>
          </div>
        )}

        {step.kind === "done" && <Results result={step.result} mode={step.mode} headingRef={headingRef} onDone={close} />}
      </DialogContent>
    </Dialog>
  );
}

/** The server's CSV parse errors start in lower case ("a quoted field is not closed (line 3)"). */
function sentence(s: string) {
  const t = s.charAt(0).toUpperCase() + s.slice(1);
  return /[.!?]$/.test(t) ? t : `${t}.`;
}

function SkippedList({ rows }: { rows: ImportSkipped[] }) {
  if (rows.length === 0) return null;
  return (
    <div>
      <p className="mb-1.5 flex items-center gap-1.5 text-xs font-medium text-foreground/80">
        <AlertCircle aria-hidden className="h-3.5 w-3.5 text-amber-700 dark:text-amber-400" />
        Skipped
      </p>
      <div className="max-h-56 overflow-y-auto rounded-lg border border-border/60">
        <table className="w-full text-left text-xs">
          <thead className="sticky top-0 bg-card text-[11px] text-muted-foreground">
            <tr className="border-b border-border/60">
              <th scope="col" className="w-14 px-3 py-1.5 font-normal">
                Row
              </th>
              <th scope="col" className="px-3 py-1.5 font-normal">
                Address
              </th>
              <th scope="col" className="px-3 py-1.5 font-normal">
                Why
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border/50">
            {rows.map((r, i) => (
              <tr key={`${r.line ?? "x"}-${r.address}-${i}`}>
                <td className="px-3 py-1.5 tabular-nums text-muted-foreground">{r.line ?? "—"}</td>
                <td className="max-w-48 break-all px-3 py-1.5 font-mono text-foreground/90">{r.address || <span className="text-muted-foreground">(empty)</span>}</td>
                <td className="px-3 py-1.5 text-foreground/80">{r.reason}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function Results({
  result,
  mode,
  headingRef,
  onDone,
}: {
  result: ImportMailboxesResponse;
  mode: Mode;
  headingRef: React.RefObject<HTMLHeadingElement | null>;
  onDone: () => void;
}) {
  const invites = mode === "invite";
  const n = result.created.length;

  function downloadCsv() {
    const stamp = new Date().toISOString().slice(0, 10).replace(/-/g, "");
    const rows = invites
      ? [["address", "invite_link", "link_expires"], ...result.created.map((c) => [c.address, c.inviteUrl, c.inviteExpiresAt])]
      : [["address", "password"], ...result.created.map((c) => [c.address, c.password])];
    saveBlob(new Blob([toCsv(rows)], { type: "text/csv;charset=utf-8" }), `mailbox-${invites ? "invites" : "passwords"}-${stamp}.csv`);
  }

  return (
    <div className="space-y-4">
      <DialogHeader>
        <DialogTitle ref={headingRef} tabIndex={-1} className="focus:outline-none">
          {n ? `${plural(n, "mailbox", "mailboxes")} created` : "No mailboxes were created"}
        </DialogTitle>
        <DialogDescription>
          {n === 0
            ? "Nothing was created — see why below."
            : invites
              ? "Send each person their link through a channel you trust. They choose a password and set up their devices. The links are shown only now."
              : "Pass each password on through a channel you trust. They are shown only now — the server keeps no readable copy."}
        </DialogDescription>
      </DialogHeader>

      {n > 0 && (
        <div className="max-h-72 overflow-y-auto rounded-lg border border-border/60">
          <ul className="divide-y divide-border/50">
            {result.created.map((c) => {
              const secret = invites ? c.inviteUrl : c.password;
              return (
                <li key={c.address} className="px-3 py-2">
                  <p className="text-xs font-medium text-foreground/90">{c.address}</p>
                  {secret && (
                    <div className="mt-1 flex items-center gap-1.5">
                      <code className="min-w-0 flex-1 select-all truncate rounded-md border border-border bg-muted/40 px-2 py-1 font-mono text-[11px] text-foreground/90" title={secret}>
                        {secret}
                      </code>
                      <CopyButton value={secret} label={`${invites ? "invite link" : "password"} for ${c.address}`} />
                    </div>
                  )}
                  {invites && c.inviteExpiresAt && <p className="mt-0.5 text-[11px] text-muted-foreground">Works once, until {fmtDate(c.inviteExpiresAt)}</p>}
                </li>
              );
            })}
          </ul>
        </div>
      )}
      <SkippedList rows={result.skipped} />

      <DialogFooter className="flex-wrap">
        {n > 0 && (
          <Button type="button" variant="outline" onClick={downloadCsv} className="mr-auto">
            <Download className="h-4 w-4" />
            Download as CSV
          </Button>
        )}
        <Button type="button" onClick={onDone}>
          {n > 0 ? "I've saved them" : "Close"}
        </Button>
      </DialogFooter>
    </div>
  );
}
