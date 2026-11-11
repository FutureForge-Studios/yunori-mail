/**
 * Status tones. Colour is reserved for state and always ships with an icon
 * and/or a word (WCAG 1.4.1). Inks come from the yunori.email tokens
 * (--success-ink, --warning-ink, --info-ink, --danger-ink), which are tuned
 * per theme to clear 4.5:1 on cards and canvas.
 */
export type Tone = "ok" | "warn" | "bad" | "info" | "muted";

export const TONE_TEXT: Record<Tone, string> = {
  ok: "text-success-ink",
  warn: "text-warning-ink",
  bad: "text-danger-ink",
  info: "text-info-ink",
  muted: "text-muted-foreground",
};

/** Soft fill + hairline for badges and callouts. */
export const TONE_SOFT: Record<Tone, string> = {
  ok: "bg-success/10 border-success/25 text-success-ink",
  warn: "bg-warning/10 border-warning/30 text-warning-ink",
  bad: "bg-destructive/10 border-destructive/25 text-danger-ink",
  info: "bg-info/10 border-info/25 text-info-ink",
  muted: "bg-muted/70 border-border text-muted-foreground",
};

export const TONE_DOT: Record<Tone, string> = {
  ok: "bg-success",
  warn: "bg-warning",
  bad: "bg-destructive",
  info: "bg-info",
  muted: "bg-muted-foreground/50",
};

/** Solid fill for meters and chart marks. */
export const TONE_FILL: Record<Tone, string> = {
  ok: "bg-success",
  warn: "bg-warning",
  bad: "bg-destructive",
  info: "bg-info",
  muted: "bg-foreground/55",
};

/** SVG stroke colours (for rings). */
export const TONE_STROKE: Record<Tone, string> = {
  ok: "hsl(var(--success))",
  warn: "hsl(var(--warning))",
  bad: "hsl(var(--destructive))",
  info: "hsl(var(--info))",
  muted: "hsl(var(--foreground) / 0.6)",
};
