import {
  AtSign,
  Bell,
  Gauge,
  Globe,
  Inbox,
  MailCheck,
  Send,
  ServerCog,
  ShieldCheck,
  type LucideIcon,
} from "lucide-react";

/**
 * The console's information architecture — one source for the sidebar, the
 * mobile drawer, breadcrumbs and the ⌘K palette (same shape as the client's
 * admin-nav.ts). Plan §9.2; Backups joins "Server" when its API exists.
 */
export interface NavItem {
  to: string;
  label: string;
  icon: LucideIcon;
  end?: boolean;
  /** Extra palette search terms. */
  keywords?: string[];
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

export const NAV: NavGroup[] = [
  {
    label: "Overview",
    items: [{ to: "/console", label: "Overview", icon: Gauge, end: true, keywords: ["home", "status", "health", "setup", "install"] }],
  },
  {
    label: "Mail",
    items: [
      { to: "/console/domains", label: "Domains", icon: Globe, keywords: ["dns", "mx", "spf", "dkim", "dmarc", "records", "catch-all"] },
      { to: "/console/mailboxes", label: "Mailboxes", icon: Inbox, keywords: ["users", "accounts", "quota", "password"] },
      { to: "/console/aliases", label: "Aliases", icon: AtSign, keywords: ["forward", "redirect"] },
    ],
  },
  {
    label: "Delivery",
    items: [
      { to: "/console/sending", label: "Sending", icon: Send, keywords: ["relay", "smarthost", "port 25", "direct", "ses", "postmark"] },
      { to: "/console/deliverability", label: "Deliverability", icon: MailCheck, keywords: ["queue", "blocklist", "ptr", "dmarc", "spam"] },
    ],
  },
  {
    label: "Server",
    items: [
      { to: "/console/security", label: "Security", icon: ShieldCheck, keywords: ["audit", "admins", "2fa", "totp", "ledger"] },
      { to: "/console/system", label: "System", icon: ServerCog, keywords: ["services", "versions", "certificate", "preflight", "disk", "memory"] },
      { to: "/console/alerts", label: "Alerts", icon: Bell, keywords: ["notifications", "email", "webhook", "ntfy", "monitoring", "warnings"] },
    ],
  },
];

/** The group + item a pathname belongs to (longest matching prefix wins). */
export function navLocation(pathname: string): { group: NavGroup; item: NavItem } | null {
  const path = pathname.replace(/\/+$/, "") || "/";
  let best: { group: NavGroup; item: NavItem } | null = null;
  for (const group of NAV) {
    for (const item of group.items) {
      const match = item.end ? path === item.to : path === item.to || path.startsWith(`${item.to}/`);
      if (match && (!best || item.to.length > best.item.to.length)) best = { group, item };
    }
  }
  return best;
}
