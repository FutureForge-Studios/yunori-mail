import type { Config } from "tailwindcss";
import yunori from "./src/design/vendor/tailwind.config";

/**
 * Structure (radii, animation plugin, base colour names) comes from the client's
 * vendored preset; the LOOK comes from yunori.email's tokens
 * (src/design/yunori/tokens.css, synced by scripts/sync-yunori-design.mjs).
 *
 * The vendored kit writes status tones as emerald/amber utilities; on
 * yunori.email those are the success/warning tokens, so the two palettes are
 * remapped here and every status follows the live Yunori palette per theme.
 */
const tone = (name: string) => `hsl(var(--${name}) / <alpha-value>)`;

export default {
  presets: [yunori],
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      fontFamily: {
        mono: ["var(--font-mono)", "ui-monospace", "SFMono-Regular", "monospace"],
      },
      colors: {
        surface: tone("surface"),
        raised: tone("raised"),
        overlay: tone("overlay"),
        "border-strong": tone("border-strong"),
        "torii-ink": tone("torii-ink"),
        success: { DEFAULT: tone("success"), ink: "var(--success-ink)" },
        warning: { DEFAULT: tone("warning"), ink: "var(--warning-ink)" },
        info: { DEFAULT: tone("info"), ink: "var(--info-ink)" },
        danger: { ink: "var(--danger-ink)", tint: "var(--danger-tint)" },
        icon: "var(--icon)",
        emerald: { 400: "var(--success-ink)", 600: tone("success"), 700: "var(--success-ink)" },
        amber: { 400: "var(--warning-ink)", 600: tone("warning"), 700: "var(--warning-ink)" },
      },
      boxShadow: {
        panel: "var(--shadow-panel)",
        pop: "var(--shadow-pop)",
      },
      transitionDuration: {
        fast: "var(--dur-fast)",
        base: "var(--dur-base)",
        slow: "var(--dur-slow)",
      },
    },
  },
} satisfies Config;
