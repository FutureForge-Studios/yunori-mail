import { useState, type ReactNode } from "react";
import { AlertTriangle } from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/ui/overlay";
import { Button } from "@/ui/button";
import { Input, Label } from "@/ui/form";
import { errorMessage } from "@/api/client";

/**
 * The console's confirmation surface — modeled on the client's admin
 * ConfirmDialog (same layout, type-to-confirm), plus inline errors: a 409 from
 * the server ("still has mailboxes") stays in the dialog instead of vanishing.
 */
export function ConfirmAction({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  destructive,
  typeToConfirm,
  onConfirm,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: ReactNode;
  confirmLabel: string;
  destructive?: boolean;
  typeToConfirm?: string;
  /** Resolve to close; throw to keep the dialog open with the error shown. */
  onConfirm: () => Promise<void>;
  children?: ReactNode;
}) {
  const [typed, setTyped] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setTyped("");
    setError(null);
  };
  const blocked = busy || (typeToConfirm !== undefined && typed.trim().toLowerCase() !== typeToConfirm.toLowerCase());

  async function confirm() {
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
      onOpenChange(false);
      reset();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (busy) return;
        onOpenChange(o);
        if (!o) reset();
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          {description && <DialogDescription>{description}</DialogDescription>}
        </DialogHeader>

        {children}

        {typeToConfirm !== undefined && (
          <form
            className="space-y-1.5"
            onSubmit={(e) => {
              e.preventDefault();
              if (!blocked) void confirm();
            }}
          >
            <Label htmlFor="confirm-type">
              Type <span className="font-medium text-foreground">{typeToConfirm}</span> to confirm
            </Label>
            <Input
              id="confirm-type"
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              autoComplete="off"
              spellCheck={false}
              autoCapitalize="none"
            />
          </form>
        )}

        {error && (
          <p role="alert" className="flex items-start gap-2 rounded-lg border border-warning/30 bg-warning/10 px-3 py-2.5 text-[13px] leading-relaxed">
            <AlertTriangle aria-hidden className="mt-0.5 h-4 w-4 shrink-0 text-warning-ink" />
            {error}
          </p>
        )}

        <DialogFooter>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
            Cancel
          </Button>
          <Button variant={destructive ? "destructive" : "default"} onClick={() => void confirm()} disabled={blocked} loading={busy}>
            {busy ? "Working…" : confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
