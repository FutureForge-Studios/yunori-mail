import { forwardRef, type ComponentPropsWithoutRef, type ElementRef, type ReactNode } from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import {
  Dialog,
  DialogClose,
  DialogContent as VendorDialogContent,
  DialogDescription,
  DialogFooter as VendorDialogFooter,
  DialogHeader,
  DialogTitle as VendorDialogTitle,
  DialogTrigger,
} from "@/design/vendor/ui/dialog";

export { Dialog, DialogClose, DialogDescription, DialogHeader, DialogTrigger };

/**
 * The vendored dialog with phone-safe sizing: a gutter at the edges, never
 * taller than the screen (the body scrolls), a quieter overlay.
 */
export const DialogContent = forwardRef<ElementRef<typeof VendorDialogContent>, ComponentPropsWithoutRef<typeof VendorDialogContent>>(
  ({ className, ...props }, ref) => (
    <VendorDialogContent
      ref={ref}
      className={cn(
        "grid max-h-[calc(100dvh-2rem)] w-[calc(100%-2rem)] gap-4 overflow-y-auto rounded-2xl bg-popover p-5 shadow-pop sm:p-6 [&>button:last-child]:right-3.5 [&>button:last-child]:top-3.5 [&>button:last-child]:grid [&>button:last-child]:h-8 [&>button:last-child]:w-8 [&>button:last-child]:place-items-center [&>button:last-child]:rounded-lg [&>button:last-child]:hover:bg-accent",
        className,
      )}
      {...props}
    />
  ),
);
DialogContent.displayName = "DialogContent";

export const DialogTitle = forwardRef<ElementRef<typeof VendorDialogTitle>, ComponentPropsWithoutRef<typeof VendorDialogTitle>>(({ className, ...props }, ref) => (
  <VendorDialogTitle ref={ref} className={cn("pr-8 font-serif text-xl leading-snug tracking-[-0.01em]", className)} {...props} />
));
DialogTitle.displayName = "DialogTitle";

export function DialogFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return <VendorDialogFooter className={cn("flex-col-reverse gap-2 pt-3 sm:flex-row", className)} {...props} />;
}

/** Slide-over panel from the left (mobile navigation) or right (details). */
export function Sheet({
  open,
  onOpenChange,
  side = "left",
  title,
  children,
  className,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  side?: "left" | "right";
  title: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-[hsl(var(--scrim)/var(--scrim-opacity))] data-[state=closed]:animate-out data-[state=open]:animate-in data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content
          aria-describedby={undefined}
          className={cn(
            "fixed inset-y-0 z-50 flex w-[min(19rem,calc(100vw-3rem))] flex-col bg-background shadow-pop outline-none",
            side === "left"
              ? "left-0 border-r border-border data-[state=closed]:animate-slide-out-left data-[state=open]:animate-slide-in-left"
              : "right-0 border-l border-border",
            className,
          )}
        >
          <DialogPrimitive.Title className="sr-only">{title}</DialogPrimitive.Title>
          {children}
          <DialogPrimitive.Close
            aria-label="Close"
            className="absolute right-3 top-3 grid h-8 w-8 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
          >
            <X className="h-4 w-4" />
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
