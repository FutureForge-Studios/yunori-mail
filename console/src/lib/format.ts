export { fmtBytes, fmtDate, fmtDateTime, fmtNumber, timeAgo } from "@/design/vendor/admin/ui";

/** "in 64 days" / "3 days ago" for certificate expiry and similar. */
export function relativeDays(iso: string | null | undefined): string {
  if (!iso) return "—";
  const days = Math.round((new Date(iso).getTime() - Date.now()) / 86_400_000);
  if (days === 0) return "today";
  if (days > 0) return `in ${days} day${days === 1 ? "" : "s"}`;
  return `${-days} day${days === -1 ? "" : "s"} ago`;
}

/** Percent (0–100) of used/total; null when either is unknown or total is 0. */
export function pct(used: number | null | undefined, total: number | null | undefined): number | null {
  if (used == null || total == null || total <= 0) return null;
  return Math.min(100, Math.max(0, (used / total) * 100));
}

/** The client's "Open in Yunori" deep link (src/lib/deep-link.ts `connectPath`). */
export function openInYunoriUrl(clientOrigin: string, address: string): string {
  return `${clientOrigin.replace(/\/+$/, "")}/welcome?connect=${encodeURIComponent(address)}`;
}
