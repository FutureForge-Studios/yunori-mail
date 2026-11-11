import { forwardRef } from "react";
import { Button as VendorButton, type ButtonProps as VendorButtonProps } from "@/design/vendor/ui/button";
import { cn } from "@/design/vendor/lib/utils";

/**
 * The vendored client button, tuned for the console: softer radius, a 2px
 * focus ring with offset (AA-visible on both canvases), outline buttons sit
 * on the card surface, and a `loading` state that keeps the width.
 */
export interface ButtonProps extends VendorButtonProps {
  loading?: boolean;
}

const VARIANT_EXTRA: Record<string, string> = {
  default: "",
  torii: "",
  outline: "bg-card hover:bg-accent shadow-card",
  ghost: "text-foreground/80 hover:text-foreground",
  secondary: "",
  destructive: "",
  link: "h-auto px-0",
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(({ className, variant, loading, disabled, children, ...props }, ref) => (
  <VendorButton
    ref={ref}
    variant={variant}
    disabled={disabled || loading}
    aria-busy={loading || undefined}
    className={cn(
      "rounded-lg focus-visible:ring-2 focus-visible:ring-ring/70 focus-visible:ring-offset-2 focus-visible:ring-offset-background",
      VARIANT_EXTRA[variant ?? "default"],
      className,
    )}
    {...props}
  >
    {props.asChild ? (
      children
    ) : (
      <>
        {loading && <span aria-hidden className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-r-transparent motion-reduce:animate-none" />}
        {children}
      </>
    )}
  </VendorButton>
));
Button.displayName = "Button";

/** Square icon-only button with an accessible name. */
export const IconButton = forwardRef<HTMLButtonElement, ButtonProps & { label: string }>(({ label, className, variant = "ghost", size, ...props }, ref) => (
  <Button
    ref={ref}
    variant={variant}
    size={size ?? "icon"}
    aria-label={label}
    title={label}
    className={cn("h-8 w-8 text-muted-foreground hover:text-foreground", className)}
    {...props}
  />
));
IconButton.displayName = "IconButton";
