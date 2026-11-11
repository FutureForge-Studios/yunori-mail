import { Suspense, createContext, useCallback, useContext, useEffect, useState } from "react";
import { Link, NavLink, Outlet, useLocation, useNavigate } from "react-router-dom";
import { ChevronRight, ChevronsUpDown, LogOut, Menu, Monitor, Moon, PanelLeftClose, PanelLeftOpen, Search, ShieldCheck, Sun } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { YunoriMark } from "@/components/yunori-logo";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/design/vendor/ui/dropdown-menu";
import { api } from "@/api/client";
import type { Check, OverviewResponse, VerdictLevel } from "@/api/types";
import { CHECK_META, toneDot, toneText, type Tone } from "@/components/status";
import { ThemeToggle, iconButton } from "@/components/theme-toggle";
import { useTheme } from "@/lib/theme";
import { Avatar, Kbd } from "@/ui/badge";
import { Skeleton } from "@/ui/feedback";
import { PageContainer } from "@/ui/page";
import { Sheet } from "@/ui/overlay";
import { NAV, navLocation, type NavGroup, type NavItem } from "./nav";
import { Palette } from "./palette";
import { useSession } from "./session";

const COLLAPSE_KEY = "yunori:console:sidebar-collapsed";

function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSE_KEY) === "1";
  } catch {
    return false;
  }
}
function writeCollapsed(v: boolean) {
  try {
    localStorage.setItem(COLLAPSE_KEY, v ? "1" : "0");
  } catch {
    /* private mode — preference just won't persist */
  }
}

// ── Shell context: top-bar health + breadcrumb detail ───────────────────────

interface ShellCtx {
  health: OverviewResponse | null;
  refreshHealth: () => void;
  setCrumb: (label: string | null) => void;
}
const ShellContext = createContext<ShellCtx>({ health: null, refreshHealth: () => {}, setCrumb: () => {} });
export const useShell = () => useContext(ShellContext);

/** Name the detail crumb (e.g. a domain) while a detail page is mounted. */
export function useCrumb(label: string | null | undefined) {
  const { setCrumb } = useShell();
  useEffect(() => {
    setCrumb(label ?? null);
    return () => setCrumb(null);
  }, [label, setCrumb]);
}

export const VERDICT_META: Record<VerdictLevel, { tone: Tone; label: string }> = {
  ok: { tone: "ok", label: "Healthy" },
  attention: { tone: "warn", label: "Needs attention" },
  degraded: { tone: "bad", label: "Degraded" },
  down: { tone: "bad", label: "Down" },
};

export function roleLabel(role: string): string {
  return role === "owner" ? "Owner" : role === "admin" ? "Admin" : role === "viewer" ? "Viewer" : role;
}

/**
 * The console shell: a pinned, collapsible sidebar (⌘\) with grouped
 * sections and the account at its foot; a sticky top bar with breadcrumbs,
 * search (⌘K), server health and theme; a slide-over drawer on phones. The
 * document scrolls (not an inner pane), so the browser's own scroll,
 * find-in-page and zoom all behave.
 */
export function ConsoleShell() {
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const [health, setHealth] = useState<OverviewResponse | null>(null);
  const [crumb, setCrumb] = useState<string | null>(null);
  const { pathname } = useLocation();

  const refreshHealth = useCallback(() => {
    api
      .overview()
      .then(setHealth)
      .catch(() => {
        /* the pill just keeps its last value */
      });
  }, []);

  useEffect(() => {
    refreshHealth();
    const t = setInterval(() => {
      if (document.visibilityState === "visible") refreshHealth();
    }, 60_000);
    return () => clearInterval(t);
  }, [refreshHealth]);

  // New page: back to the top (the document scrolls, not an inner pane).
  useEffect(() => {
    window.scrollTo({ top: 0 });
  }, [pathname]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      } else if (e.key === "\\") {
        e.preventDefault();
        setCollapsed((c) => {
          writeCollapsed(!c);
          return !c;
        });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const toggleCollapsed = () => {
    writeCollapsed(!collapsed);
    setCollapsed(!collapsed);
  };

  return (
    <ShellContext.Provider value={{ health, refreshHealth, setCrumb }}>
      <div className="min-h-dvh bg-background lg:flex">
        <a
          href="#main"
          className="sr-only z-[80] rounded-lg bg-card px-3 py-2 text-sm shadow-pop focus:not-sr-only focus:fixed focus:left-3 focus:top-3 focus:outline-none focus:ring-2 focus:ring-ring/60"
        >
          Skip to content
        </a>

        {/* Sidebar: pinned to the viewport from lg up. */}
        <aside
          aria-label="Console"
          className={cn(
            "sticky top-0 hidden h-dvh shrink-0 flex-col border-r border-border bg-background transition-[width] duration-200 ease-out motion-reduce:transition-none lg:flex",
            collapsed ? "w-[var(--sidebar-width-collapsed)]" : "w-[var(--sidebar-width)]",
          )}
        >
          <SidebarContents collapsed={collapsed} health={health} onSearch={() => setPaletteOpen(true)} onToggleCollapsed={toggleCollapsed} />
        </aside>

        {/* Main column */}
        <div className="flex min-w-0 flex-1 flex-col">
          <TopBar crumb={crumb} health={health} onMenu={() => setDrawerOpen(true)} onSearch={() => setPaletteOpen(true)} />
          <main id="main" tabIndex={-1} className="min-w-0 flex-1 focus:outline-none">
            <Suspense fallback={<PageFallback />}>
              <Outlet />
            </Suspense>
          </main>
        </div>

        {/* Phones and tablets: the same sidebar as a drawer. */}
        <Sheet open={drawerOpen} onOpenChange={setDrawerOpen} title="Console navigation">
          <SidebarContents collapsed={false} health={health} onSearch={() => {
            setDrawerOpen(false);
            setPaletteOpen(true);
          }} onNavigate={() => setDrawerOpen(false)} />
        </Sheet>

        <Palette open={paletteOpen} onOpenChange={setPaletteOpen} />
      </div>
    </ShellContext.Provider>
  );
}

function PageFallback() {
  return (
    <PageContainer>
      <div aria-busy="true">
        <Skeleton className="h-4 w-32" />
        <Skeleton className="mt-3 h-9 w-72" />
        <div className="mt-8 grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <Skeleton key={i} className="h-28 rounded-xl" />
          ))}
        </div>
        <Skeleton className="mt-4 h-80 rounded-xl" />
      </div>
    </PageContainer>
  );
}

// ── Sidebar ─────────────────────────────────────────────────────────────────

function SidebarContents({
  collapsed,
  health,
  onSearch,
  onNavigate,
  onToggleCollapsed,
}: {
  collapsed: boolean;
  health: OverviewResponse | null;
  onSearch: () => void;
  onNavigate?: () => void;
  onToggleCollapsed?: () => void;
}) {
  const { session } = useSession();
  const meta = health ? VERDICT_META[health.verdict.level] : null;
  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* Brand + server */}
      <div className={cn("flex h-[var(--topbar-height)] shrink-0 items-center gap-2.5", collapsed ? "justify-center px-2" : "px-4")}>
        <Link
          to="/console"
          onClick={onNavigate}
          aria-label="Yunori Mail — overview"
          className="grid h-8 w-8 shrink-0 place-items-center rounded-lg border border-border bg-card shadow-card focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60"
        >
          <YunoriMark className="h-[18px] w-[18px]" />
        </Link>
        {!collapsed && (
          <div className="min-w-0 leading-tight">
            <p className="text-sm font-semibold tracking-tight">Yunori Mail</p>
            <p className="flex items-center gap-1.5 truncate text-2xs text-muted-foreground" title={meta ? `${session.server.hostname} · ${meta.label}` : session.server.hostname}>
              <span aria-hidden className={cn("h-1.5 w-1.5 shrink-0 rounded-full", meta ? toneDot(meta.tone) : "bg-muted-foreground/40")} />
              <span className="truncate font-mono">{session.server.hostname}</span>
            </p>
          </div>
        )}
      </div>

      {/* Search */}
      <div className={cn("shrink-0 pb-2", collapsed ? "px-3" : "px-3")}>
        <button
          type="button"
          onClick={onSearch}
          aria-label="Search the console"
          aria-keyshortcuts="Meta+K Control+K"
          title={collapsed ? "Search (⌘K)" : undefined}
          className={cn(
            "flex h-9 w-full items-center gap-2 rounded-lg border border-border bg-card text-[13px] text-muted-foreground shadow-card transition-colors hover:border-border-strong hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60",
            collapsed ? "justify-center px-0" : "px-2.5",
          )}
        >
          <Search className="h-4 w-4 shrink-0" />
          {!collapsed && (
            <>
              <span className="flex-1 text-left">Search…</span>
              <Kbd>⌘K</Kbd>
            </>
          )}
        </button>
      </div>

      <SidebarNav groups={NAV} collapsed={collapsed} onNavigate={onNavigate} />

      {/* Foot: account + collapse, pinned to the bottom. */}
      <div className={cn("mt-auto shrink-0 border-t border-border p-2", collapsed && "flex flex-col items-center gap-1")}>
        <AccountMenu collapsed={collapsed} />
        {onToggleCollapsed && (
          <button
            type="button"
            onClick={onToggleCollapsed}
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-keyshortcuts="Meta+Backslash Control+Backslash"
            title={`${collapsed ? "Expand" : "Collapse"} sidebar (⌘\\)`}
            className={cn(
              "flex items-center gap-2 rounded-lg text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60",
              collapsed ? "h-9 w-9 justify-center" : "mt-1 h-8 w-full px-2.5",
            )}
          >
            {collapsed ? <PanelLeftOpen className="h-4 w-4" /> : <PanelLeftClose className="h-4 w-4" />}
            {!collapsed && (
              <>
                <span className="flex-1 text-left">Collapse</span>
                <Kbd>⌘\</Kbd>
              </>
            )}
          </button>
        )}
      </div>
    </div>
  );
}

function SidebarNav({ groups, collapsed, onNavigate }: { groups: NavGroup[]; collapsed: boolean; onNavigate?: () => void }) {
  return (
    <nav className={cn("min-h-0 flex-1 overflow-y-auto pb-3 pt-1", collapsed ? "px-3" : "px-3")} aria-label="Console sections">
      {groups.map((group, gi) => (
        <div key={group.label} className={cn(gi > 0 && "mt-4")}>
          {collapsed ? (
            gi > 0 && <div aria-hidden className="mx-2 mb-3 border-t border-border" />
          ) : (
            group.items.length > 1 || group.label !== group.items[0]?.label ? (
              <p className="mb-1 px-2.5 text-2xs font-semibold uppercase tracking-[0.1em] text-muted-foreground/90">{group.label}</p>
            ) : null
          )}
          <ul className="space-y-0.5">
            {group.items.map((item) => (
              <li key={item.to}>
                <SidebarLink item={item} collapsed={collapsed} onNavigate={onNavigate} />
              </li>
            ))}
          </ul>
        </div>
      ))}
    </nav>
  );
}

function SidebarLink({ item, collapsed, onNavigate }: { item: NavItem; collapsed: boolean; onNavigate?: () => void }) {
  return (
    <NavLink
      to={item.to}
      end={item.end}
      onClick={onNavigate}
      title={collapsed ? item.label : undefined}
      aria-label={collapsed ? item.label : undefined}
      className={({ isActive }) =>
        cn(
          "group relative flex h-9 items-center gap-2.5 rounded-lg text-[13.5px] transition-colors duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60",
          collapsed ? "justify-center px-0" : "px-2.5",
          isActive
            ? "bg-card font-medium text-foreground shadow-card ring-1 ring-border"
            : "text-foreground/70 hover:bg-accent/70 hover:text-foreground",
        )
      }
    >
      {({ isActive }) => (
        <>
          <span
            aria-hidden
            className={cn("absolute -left-3 top-1/2 h-4 w-[3px] -translate-y-1/2 rounded-r-full bg-torii transition-opacity", isActive ? "opacity-100" : "opacity-0")}
          />
          <item.icon strokeWidth={1.75} className={cn("h-[18px] w-[18px] shrink-0 transition-colors", isActive ? "text-foreground" : "text-icon group-hover:text-foreground")} />
          {!collapsed && <span className="truncate">{item.label}</span>}
        </>
      )}
    </NavLink>
  );
}

// ── Top bar ─────────────────────────────────────────────────────────────────

function TopBar({ crumb, health, onMenu, onSearch }: { crumb: string | null; health: OverviewResponse | null; onMenu: () => void; onSearch: () => void }) {
  const { pathname } = useLocation();
  const { session } = useSession();
  const here = navLocation(pathname);
  const atItem = here && pathname.replace(/\/+$/, "") === here.item.to;
  return (
    <header className="sticky top-0 z-30 flex h-[var(--topbar-height)] shrink-0 items-center gap-2 border-b border-border bg-background/85 px-3 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 sm:px-5 lg:px-8">
      <button type="button" onClick={onMenu} aria-label="Open navigation" className={cn(iconButton, "-ml-1 lg:hidden")}>
        <Menu className="h-[18px] w-[18px]" />
      </button>
      <Link to="/console" aria-label="Overview" className="flex items-center gap-2 rounded-md focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 lg:hidden">
        <YunoriMark className="h-5 w-5" />
        <span className="text-sm font-semibold tracking-tight sm:hidden">{here?.item.label ?? "Yunori Mail"}</span>
      </Link>

      <nav aria-label="Breadcrumb" className="hidden min-w-0 sm:block">
        <ol className="flex min-w-0 items-center gap-1.5 text-[13px]">
          <li className="hidden shrink-0 text-muted-foreground lg:block">Console</li>
          {here && (
            <>
              <li aria-hidden className="hidden lg:block">
                <ChevronRight className="h-3.5 w-3.5 text-muted-foreground/60" />
              </li>
              {here.group.label !== here.item.label && (
                <>
                  <li className="shrink-0 text-muted-foreground">{here.group.label}</li>
                  <li aria-hidden>
                    <ChevronRight className="h-3.5 w-3.5 text-muted-foreground/60" />
                  </li>
                </>
              )}
              <li className="min-w-0 truncate">
                {atItem ? (
                  <span aria-current="page" className="font-medium text-foreground">
                    {here.item.label}
                  </span>
                ) : (
                  <Link to={here.item.to} className="rounded-sm text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60">
                    {here.item.label}
                  </Link>
                )}
              </li>
              {!atItem && (
                <>
                  <li aria-hidden>
                    <ChevronRight className="h-3.5 w-3.5 text-muted-foreground/60" />
                  </li>
                  <li className="min-w-0 truncate font-medium text-foreground" aria-current="page">
                    {crumb ?? "Detail"}
                  </li>
                </>
              )}
            </>
          )}
        </ol>
      </nav>

      <div className="ml-auto flex items-center gap-1.5">
        <button
          type="button"
          onClick={onSearch}
          aria-label="Search the console"
          aria-keyshortcuts="Meta+K Control+K"
          className="hidden h-8 w-56 items-center gap-2 rounded-lg border border-border bg-card px-2.5 text-[13px] text-muted-foreground shadow-card transition-colors hover:border-border-strong hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 xl:flex"
        >
          <Search className="h-4 w-4" />
          <span className="flex-1 text-left">Search or jump to…</span>
          <Kbd>⌘K</Kbd>
        </button>
        <button type="button" onClick={onSearch} aria-label="Search" className={cn(iconButton, "xl:hidden")}>
          <Search className="h-4 w-4" />
        </button>
        <HealthMenu health={health} hostname={session.server.hostname} />
        <ThemeToggle />
        <div className="lg:hidden">
          <AccountMenu compact />
        </div>
      </div>
    </header>
  );
}

// ── Top-bar menus ───────────────────────────────────────────────────────────

export const SIGNAL_TARGET: Record<string, string> = {
  dns: "/console/domains",
  ptr: "/console/deliverability",
  tls: "/console/system",
  port25: "/console/sending",
  blocklists: "/console/deliverability",
  engine: "/console/system",
  queue: "/console/queue",
};

function HealthMenu({ health, hostname }: { health: OverviewResponse | null; hostname: string }) {
  const navigate = useNavigate();
  if (!health) {
    return (
      <span className="hidden h-8 items-center gap-1.5 rounded-full border border-border px-3 text-xs text-muted-foreground md:inline-flex">
        <span aria-hidden className="h-1.5 w-1.5 animate-pulse rounded-full bg-muted-foreground/50 motion-reduce:animate-none" />
        Checking…
      </span>
    );
  }
  const meta = VERDICT_META[health.verdict.level];
  const tone: Tone = health.setup ? "warn" : meta.tone;
  const issues: Check[] = health.signals.filter((s) => s.status === "fail" || s.status === "warn");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={`${hostname} health: ${meta.label}. ${health.verdict.sentence}`}
        className="inline-flex h-8 items-center gap-2 rounded-full border border-border bg-card px-2.5 text-xs font-medium text-foreground/85 shadow-card transition-colors hover:border-border-strong hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 sm:px-3"
      >
        <span aria-hidden className="relative flex h-2 w-2">
          {tone !== "ok" && <span className={cn("absolute inline-flex h-full w-full animate-ping rounded-full opacity-50 motion-reduce:hidden", toneDot(tone))} />}
          <span className={cn("relative inline-flex h-2 w-2 rounded-full", toneDot(tone))} />
        </span>
        <span className="hidden sm:inline">{health.setup ? "Setting up" : meta.label}</span>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-[min(22rem,calc(100vw-1.5rem))] rounded-xl p-1.5">
        <div className="px-2.5 py-2">
          <p className={cn("text-sm font-medium", toneText(tone))}>{health.setup ? "Setting up" : meta.label}</p>
          <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{health.verdict.sentence}</p>
        </div>
        <DropdownMenuSeparator />
        {health.setup && (
          <DropdownMenuItem onSelect={() => navigate("/console")} className="text-[13px]">
            <span aria-hidden className={cn("h-2 w-2 shrink-0 rounded-full", toneDot("warn"))} />
            Setup checklist
            <span className="ml-auto text-xs text-muted-foreground">In progress</span>
          </DropdownMenuItem>
        )}
        {issues.length === 0 ? (
          <p className="flex items-center gap-2 px-2.5 py-2 text-[13px] text-muted-foreground">
            <ShieldCheck aria-hidden className="h-4 w-4 text-success-ink" />
            Every signal passes. Quiet is good.
          </p>
        ) : (
          issues.map((s) => {
            const m = CHECK_META[s.status];
            return (
              <DropdownMenuItem key={s.id} onSelect={() => navigate(SIGNAL_TARGET[s.id] ?? "/console")} className="text-[13px]">
                <m.icon aria-hidden className={cn("h-4 w-4 shrink-0", toneText(m.tone))} />
                <span className="truncate">{s.label}</span>
                <span className={cn("ml-auto shrink-0 text-xs", toneText(m.tone))}>{m.label}</span>
              </DropdownMenuItem>
            );
          })
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => navigate("/console")} className="justify-center text-[13px] font-medium">
          Open overview
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function AccountMenu({ collapsed, compact }: { collapsed?: boolean; compact?: boolean }) {
  const { session, setSession } = useSession();
  const navigate = useNavigate();
  const { theme, setTheme } = useTheme();
  const admin = session.admin!;

  async function signOut() {
    try {
      await api.logout();
    } finally {
      setSession({ ...session, authenticated: false, admin: null });
      navigate("/console/login", { replace: true });
    }
  }

  const trigger = compact ? (
    <DropdownMenuTrigger aria-label={`Account: ${admin.email}`} className="grid h-8 w-8 place-items-center rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60">
      <Avatar name={admin.email} className="h-7 w-7" />
    </DropdownMenuTrigger>
  ) : (
    <DropdownMenuTrigger
      aria-label={`Account: ${admin.email}`}
      title={collapsed ? admin.email : undefined}
      className={cn(
        "flex items-center gap-2.5 rounded-lg text-left transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60 data-[state=open]:bg-accent",
        collapsed ? "h-9 w-9 justify-center" : "w-full px-2 py-1.5",
      )}
    >
      <Avatar name={admin.email} className="h-7 w-7" />
      {!collapsed && (
        <>
          <span className="min-w-0 flex-1 leading-tight">
            <span className="block truncate text-[13px] font-medium text-foreground">{admin.email}</span>
            <span className="block truncate text-2xs text-muted-foreground">
              {roleLabel(admin.role)}
              {admin.totpEnabled ? " · 2FA on" : " · 2FA off"}
            </span>
          </span>
          <ChevronsUpDown aria-hidden className="h-4 w-4 shrink-0 text-muted-foreground" />
        </>
      )}
    </DropdownMenuTrigger>
  );

  const themes = [
    { value: "light", label: "Light", icon: Sun },
    { value: "dark", label: "Dark", icon: Moon },
    { value: "system", label: "System", icon: Monitor },
  ] as const;

  return (
    <DropdownMenu>
      {trigger}
      <DropdownMenuContent align={compact ? "end" : "start"} side={compact ? "bottom" : "top"} className="w-64 rounded-xl p-1.5">
        <div className="flex items-center gap-2.5 px-2 py-2">
          <Avatar name={admin.email} />
          <div className="min-w-0">
            <p className="truncate text-[13px] font-medium">{admin.email}</p>
            <p className="text-xs text-muted-foreground">
              {roleLabel(admin.role)} · two-step sign-in {admin.totpEnabled ? "on" : "off"}
            </p>
          </div>
        </div>
        <DropdownMenuSeparator />
        <div className="px-2 py-1.5">
          <p className="mb-1.5 text-2xs font-semibold uppercase tracking-[0.08em] text-muted-foreground">Theme</p>
          <div role="radiogroup" aria-label="Theme" className="grid grid-cols-3 gap-1 rounded-lg bg-muted/70 p-0.5">
            {themes.map((t) => (
              <button
                key={t.value}
                type="button"
                role="radio"
                aria-checked={theme === t.value}
                onClick={() => setTheme(t.value)}
                className={cn(
                  "flex h-7 items-center justify-center gap-1 rounded-md text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/60",
                  theme === t.value ? "bg-card font-medium text-foreground shadow-card" : "text-muted-foreground hover:text-foreground",
                )}
              >
                <t.icon aria-hidden className="h-3.5 w-3.5" />
                {t.label}
              </button>
            ))}
          </div>
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => navigate("/console/security")} className="text-[13px]">
          <ShieldCheck className="h-4 w-4 text-icon" aria-hidden />
          Admins & audit
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => void signOut()} className="text-[13px]">
          <LogOut className="h-4 w-4 text-icon" aria-hidden />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
