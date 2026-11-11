import { forwardRef, useState, type InputHTMLAttributes } from "react";
import { Eye, EyeOff } from "lucide-react";
import { Input } from "@/ui/form";
import { cn } from "@/design/vendor/lib/utils";
import { passwordStrength } from "@/lib/password";

/** Password field with the client's show/hide affordance. */
export const PasswordInput = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(
  ({ className, ...props }, ref) => {
    const [show, setShow] = useState(false);
    return (
      <div className="relative">
        <Input ref={ref} type={show ? "text" : "password"} className={cn("pr-10", className)} {...props} />
        <button
          type="button"
          onClick={() => setShow((v) => !v)}
          aria-label="Show password"
          aria-pressed={show}
          title={show ? "Hide password" : "Show password"}
          className="absolute right-0 top-0 grid h-full w-10 place-items-center rounded-r-lg text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/60"
        >
          {show ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
        </button>
      </div>
    );
  },
);
PasswordInput.displayName = "PasswordInput";

/** Four quiet segments + a word. Neutral ink, not traffic-light colors. */
export function StrengthHint({ password, id }: { password: string; id?: string }) {
  const s = passwordStrength(password);
  return (
    <div id={id} className="space-y-1.5" aria-live="polite">
      <div aria-hidden className="flex gap-1">
        {[1, 2, 3, 4].map((i) => (
          <span
            key={i}
            className={cn(
              "h-1 flex-1 rounded-full transition-colors motion-reduce:transition-none",
              s.score >= i ? (s.score === 1 ? "bg-warning" : s.score >= 3 ? "bg-success" : "bg-foreground/60") : "bg-muted",
            )}
          />
        ))}
      </div>
      <p className="text-xs text-muted-foreground">
        {s.label && <span className="font-medium text-foreground/80">{s.label}. </span>}
        {s.hint}
      </p>
    </div>
  );
}
