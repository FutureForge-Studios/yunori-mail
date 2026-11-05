import type { Config } from "tailwindcss";
import yunori from "./src/design/vendor/tailwind.config";

/**
 * The client's Tailwind config (vendored, see scripts/sync-design.mjs) used as
 * a preset — same tokens, radii, fonts and animation plugin. Only the content
 * globs are the console's own.
 */
export default {
  presets: [yunori],
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
} satisfies Config;
