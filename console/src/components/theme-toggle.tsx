import { Moon, Sun } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { useTheme } from "@/lib/theme";

export const iconButton =
  "grid h-8 w-8 shrink-0 place-items-center rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60";

/** Same control as the client's admin top bar. */
export function ThemeToggle({ className }: { className?: string }) {
  const { resolvedTheme, setTheme } = useTheme();
  const dark = resolvedTheme === "dark";
  return (
    <button
      type="button"
      onClick={() => setTheme(dark ? "light" : "dark")}
      aria-label={dark ? "Switch to light theme" : "Switch to dark theme"}
      title={dark ? "Light theme" : "Dark theme"}
      className={cn(iconButton, className)}
    >
      {dark ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
    </button>
  );
}
