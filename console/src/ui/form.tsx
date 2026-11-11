import { forwardRef, useId, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes } from "react";
import { Check, ChevronDown } from "lucide-react";
import { Input as VendorInput } from "@/design/vendor/ui/input";
import { Textarea as VendorTextarea } from "@/design/vendor/ui/textarea";
import { Switch as VendorSwitch } from "@/design/vendor/ui/switch";
import { cn } from "@/design/vendor/lib/utils";

/**
 * Form controls. Text fields sit on the card surface with a hairline, a
 * quiet inset and a 2px focus ring; the select is native (free keyboard,
 * screen-reader and mobile picker support) dressed to match.
 */
export const fieldSurface =
  "rounded-lg border border-input bg-card text-sm shadow-[inset_0_1px_1px_hsl(var(--shadow-tint)/var(--shadow-a1))] transition-[border-color,box-shadow] placeholder:text-muted-foreground/80 hover:border-border-strong focus-visible:border-ring/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/20 disabled:cursor-not-allowed disabled:opacity-55 aria-[invalid=true]:border-destructive/60 aria-[invalid=true]:focus-visible:ring-destructive/20";

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(({ className, ...props }, ref) => (
  <VendorInput ref={ref} className={cn("h-9", fieldSurface, className)} {...props} />
));
Input.displayName = "Input";

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaHTMLAttributes<HTMLTextAreaElement>>(({ className, ...props }, ref) => (
  <VendorTextarea ref={ref} className={cn(fieldSurface, "leading-relaxed", className)} {...props} />
));
Textarea.displayName = "Textarea";

/** Classes for a native <select> that matches <Input>. Prefer <Select>. */
export const selectClass = cn("h-9 w-full appearance-none pl-3 pr-9", fieldSurface);

export interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  /** Classes for the wrapper (width, flex). `className` styles the control. */
  wrapperClassName?: string;
}

/** Native select with a custom chevron — looks like the rest of the form. */
export const Select = forwardRef<HTMLSelectElement, SelectProps>(({ className, wrapperClassName, children, ...props }, ref) => (
  <div className={cn("relative w-full", wrapperClassName)}>
    <select ref={ref} className={cn(selectClass, "cursor-pointer", className)} {...props}>
      {children}
    </select>
    <ChevronDown aria-hidden className="pointer-events-none absolute right-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
  </div>
));
Select.displayName = "Select";

/** Field label (sentence case — the vendored Label is an uppercase eyebrow). */
export function Label({ htmlFor, children, optional, className, id }: { htmlFor?: string; children: ReactNode; optional?: boolean; className?: string; id?: string }) {
  return (
    <label htmlFor={htmlFor} id={id} className={cn("block text-[13px] font-medium text-foreground/90", className)}>
      {children}
      {optional && <span className="ml-1 font-normal text-muted-foreground">(optional)</span>}
    </label>
  );
}

/** Label + control + hint, consistently spaced. */
export function Field({
  label,
  htmlFor,
  hint,
  error,
  errorId,
  optional,
  children,
  className,
}: {
  label: ReactNode;
  htmlFor?: string;
  hint?: ReactNode;
  error?: ReactNode;
  errorId?: string;
  optional?: boolean;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("space-y-1.5", className)}>
      <Label htmlFor={htmlFor} optional={optional}>
        {label}
      </Label>
      {children}
      {error ? <FieldError id={errorId ?? `${htmlFor ?? "field"}-err`}>{error}</FieldError> : hint && <FieldHint>{hint}</FieldHint>}
    </div>
  );
}

export function FieldHint({ id, children, className }: { id?: string; children: ReactNode; className?: string }) {
  return (
    <p id={id} className={cn("text-xs leading-relaxed text-muted-foreground", className)}>
      {children}
    </p>
  );
}

/** Inline form error tied to a field via aria-describedby. */
export function FieldError({ id, children }: { id: string; children: ReactNode }) {
  if (!children) return null;
  return (
    <p id={id} role="alert" className="text-xs leading-relaxed text-danger-ink">
      {children}
    </p>
  );
}

/** The vendored switch with the console's focus ring. */
export const Switch = forwardRef<HTMLButtonElement, React.ComponentPropsWithoutRef<typeof VendorSwitch>>(({ className, ...props }, ref) => (
  <VendorSwitch
    ref={ref}
    className={cn("focus-visible:ring-2 focus-visible:ring-ring/70 focus-visible:ring-offset-2 focus-visible:ring-offset-background data-[state=unchecked]:bg-border-strong/70", className)}
    {...props}
  />
));
Switch.displayName = "Switch";

/** A setting with a title, a description and a switch on the right. */
export function SwitchRow({
  id,
  title,
  description,
  checked,
  onCheckedChange,
  disabled,
  className,
  stateText,
}: {
  id?: string;
  title: ReactNode;
  description?: ReactNode;
  checked: boolean;
  onCheckedChange: (v: boolean) => void;
  disabled?: boolean;
  className?: string;
  /** Visible On/Off text next to the switch. */
  stateText?: boolean;
}) {
  const auto = useId();
  const sid = id ?? `sw-${auto}`;
  return (
    <div className={cn("flex items-start justify-between gap-4", className)}>
      <div className="min-w-0">
        <label htmlFor={sid} className="cursor-pointer text-sm font-medium text-foreground">
          {title}
        </label>
        {description && (
          <p id={`${sid}-desc`} className="mt-0.5 text-[13px] leading-relaxed text-muted-foreground">
            {description}
          </p>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2 pt-0.5">
        {stateText && (
          <span aria-hidden className="text-xs text-muted-foreground">
            {checked ? "On" : "Off"}
          </span>
        )}
        <Switch id={sid} checked={checked} onCheckedChange={onCheckedChange} disabled={disabled} aria-describedby={description ? `${sid}-desc` : undefined} />
      </div>
    </div>
  );
}

/** Styled native checkbox. */
export const Checkbox = forwardRef<HTMLInputElement, Omit<InputHTMLAttributes<HTMLInputElement>, "type">>(({ className, ...props }, ref) => (
  <span className={cn("relative inline-grid h-4 w-4 shrink-0 place-items-center", className)}>
    <input
      ref={ref}
      type="checkbox"
      className="peer h-4 w-4 cursor-pointer appearance-none rounded-[5px] border border-border-strong bg-card shadow-card transition-colors checked:border-foreground checked:bg-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50"
      {...props}
    />
    <Check aria-hidden strokeWidth={3} className="pointer-events-none absolute h-3 w-3 text-background opacity-0 peer-checked:opacity-100" />
  </span>
));
Checkbox.displayName = "Checkbox";

/** Checkbox with label and hint. */
export function CheckboxRow({
  checked,
  onChange,
  label,
  hint,
  disabled,
  className,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: ReactNode;
  hint?: ReactNode;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <label className={cn("flex cursor-pointer items-start gap-3 text-sm", disabled && "cursor-not-allowed opacity-60", className)}>
      <Checkbox checked={checked} onChange={(e) => onChange(e.target.checked)} disabled={disabled} className="mt-0.5" />
      <span className="min-w-0">
        <span className="text-foreground/90">{label}</span>
        {hint && <span className="mt-0.5 block text-xs leading-relaxed text-muted-foreground">{hint}</span>}
      </span>
    </label>
  );
}

/** Custom radio dot used by RadioRow and RadioCard. */
function RadioDot({ checked }: { checked: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "grid h-4 w-4 shrink-0 place-items-center rounded-full border bg-card transition-colors",
        checked ? "border-foreground bg-foreground" : "border-border-strong",
      )}
    >
      {checked && <span className="h-1.5 w-1.5 rounded-full bg-background" />}
    </span>
  );
}

/** A radio choice with an optional hint under its label. */
export function RadioRow({ name, checked, onChange, label, hint }: { name: string; checked: boolean; onChange: () => void; label: ReactNode; hint?: ReactNode }) {
  return (
    <label className="group flex cursor-pointer items-start gap-3 rounded-lg text-sm has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring/60 has-[:focus-visible]:ring-offset-2 has-[:focus-visible]:ring-offset-background">
      <input type="radio" name={name} checked={checked} onChange={onChange} className="sr-only" />
      <span className="mt-0.5">
        <RadioDot checked={checked} />
      </span>
      <span className="min-w-0">
        <span className="text-foreground/90">{label}</span>
        {hint && <span className="mt-0.5 block text-xs leading-relaxed text-muted-foreground">{hint}</span>}
      </span>
    </label>
  );
}

/** A large, card-shaped radio option (sending mode, relay provider, invite method). */
export function RadioCard({
  name,
  value,
  checked,
  onChange,
  title,
  description,
  badge,
  children,
  className,
  compact,
}: {
  name: string;
  value: string;
  checked: boolean;
  onChange: () => void;
  title: ReactNode;
  description?: ReactNode;
  badge?: ReactNode;
  children?: ReactNode;
  className?: string;
  compact?: boolean;
}) {
  return (
    <label
      className={cn(
        "relative flex cursor-pointer flex-col rounded-xl border bg-card transition-[border-color,box-shadow] has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring/60 has-[:focus-visible]:ring-offset-2 has-[:focus-visible]:ring-offset-background",
        compact ? "px-3.5 py-3" : "p-5",
        checked ? "border-foreground/60 shadow-[0_0_0_1px_hsl(var(--foreground)/0.6)]" : "border-border shadow-card hover:border-border-strong",
        className,
      )}
    >
      <input type="radio" name={name} value={value} checked={checked} onChange={onChange} className="sr-only" />
      <div className="flex items-start gap-3">
        <span className="mt-0.5">
          <RadioDot checked={checked} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1">
            <span className="text-sm font-medium text-foreground">{title}</span>
            {badge}
          </div>
          {description && <p className={cn("leading-relaxed text-muted-foreground", compact ? "mt-0.5 text-xs" : "mt-1 text-[13px]")}>{description}</p>}
        </div>
      </div>
      {children}
    </label>
  );
}

/** Address composer: [local] @ [domain select]. */
export function AddressInput({
  local,
  onLocal,
  domainId,
  onDomain,
  domains,
  placeholder,
  invalid,
  describedBy,
  autoFocus,
  id,
}: {
  local: string;
  onLocal: (v: string) => void;
  domainId: string;
  onDomain: (v: string) => void;
  domains: { id: string; name: string }[];
  placeholder?: string;
  invalid?: boolean;
  describedBy?: string;
  autoFocus?: boolean;
  id?: string;
}) {
  return (
    <div className="flex items-stretch">
      <Input
        id={id}
        aria-label="Name before the @"
        value={local}
        onChange={(e) => onLocal(e.target.value)}
        placeholder={placeholder}
        autoComplete="off"
        autoCapitalize="none"
        spellCheck={false}
        autoFocus={autoFocus}
        className="min-w-0 flex-1 rounded-r-none"
        aria-invalid={invalid}
        aria-describedby={describedBy}
      />
      <span aria-hidden className="grid place-items-center border-y border-input bg-muted/60 px-2.5 text-sm text-muted-foreground">
        @
      </span>
      <Select aria-label="Domain" wrapperClassName="w-auto max-w-[55%] shrink-0" className="rounded-l-none" value={domainId} onChange={(e) => onDomain(e.target.value)}>
        {domains.map((d) => (
          <option key={d.id} value={d.id}>
            {d.name}
          </option>
        ))}
      </Select>
    </div>
  );
}
