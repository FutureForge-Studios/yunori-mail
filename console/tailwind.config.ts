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
      fontSize: {
        "2xs": ["0.6875rem", { lineHeight: "1rem" }],
      },
      screens: {
        "3xl": "1800px",
      },
      keyframes: {
        "slide-in-left": { from: { transform: "translateX(-100%)" }, to: { transform: "translateX(0)" } },
        "slide-out-left": { from: { transform: "translateX(0)" }, to: { transform: "translateX(-100%)" } },
        "toast-in": { from: { opacity: "0", transform: "translateY(8px) scale(0.98)" }, to: { opacity: "1", transform: "translateY(0) scale(1)" } },
        "rise-in": { from: { opacity: "0", transform: "translateY(6px)" }, to: { opacity: "1", transform: "translateY(0)" } },
      },
      animation: {
        "slide-in-left": "slide-in-left 0.22s cubic-bezier(.22,1,.36,1)",
        "slide-out-left": "slide-out-left 0.18s ease-in",
        "toast-in": "toast-in 0.22s cubic-bezier(.22,1,.36,1)",
        "rise-in": "rise-in 0.35s cubic-bezier(.22,1,.36,1)",
      },
      boxShadow: {
        panel: "var(--shadow-panel)",
        pop: "var(--shadow-pop)",
        card: "0 1px 2px 0 hsl(var(--shadow-tint) / var(--shadow-a1)), 0 1px 3px -1px hsl(var(--shadow-tint) / var(--shadow-a1))",
      },
      transitionDuration: {
        fast: "var(--dur-fast)",
        base: "var(--dur-base)",
        slow: "var(--dur-slow)",
      },
    },
  },
} satisfies Config;
