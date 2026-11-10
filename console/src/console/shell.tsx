import { Suspense, createContext, useCallback, useContext, useEffect, useState } from "react";
import { Link, NavLink, Outlet, useLocation, useNavigate } from "react-router-dom";
import { ChevronRight, LogOut, Menu, PanelLeftClose, PanelLeftOpen, Search, UserRound } from "lucide-react";
import { cn } from "@/design/vendor/lib/utils";
import { YunoriMark } from "@/components/yunori-logo";
import { Dialog, DialogContent, DialogTitle } from "@/design/vendor/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/design/vendor/ui/dropdown-menu";
import { Skeleton } from "@/design/vendor/admin/ui";
import { api } from "@/api/client";
import type { Check, OverviewResponse, VerdictLevel } from "@/api/types";
import { CHECK_META, toneDot, toneText, type Tone } from "@/components/status";
import { ThemeToggle, iconButton } from "@/components/theme-toggle";
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

/**
 * The console shell — the client's admin shell, structurally: collapsible
 * grouped sidebar (⌘\), top bar with breadcrumbs, server health, theme and
 * account, ⌘K palette, mobile drawer.
 */
export function ConsoleShell() {
  const { session } = useSession();
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

  const here = navLocation(pathname);
  const toggleCollapsed = () => {
    writeCollapsed(!collapsed);
    setCollapsed(!collapsed);
  };
  const admin = session.admin!;

  return (
    <ShellContext.Provider value={{ health, refreshHealth, setCrumb }}>
      <div className="flex h-dvh overflow-hidden bg-background">
        <a
          href="#main"
          className="sr-only z-[80] rounded-md bg-card px-3 py-2 text-sm focus:not-sr-only focus:fixed focus:left-3 focus:top-3 focus:ring-1 focus:ring-ring"
        >
          Skip to content
        </a>

        {/* Sidebar (static ≥ lg) */}
        <aside
          className={cn(
            "hidden shrink-0 flex-col border-r border-border/60 bg-card/30 transition-[width] duration-200 motion-reduce:transition-none lg:flex",
            collapsed ? "w-[60px]" : "w-60",
          )}
        >
          <div className={cn("flex items-center gap-2.5 pb-4 pt-5", collapsed ? "justify-center px-2" : "px-5")}>
            <Link to="/console" aria-label="Overview" className="shrink-0 rounded-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
              <YunoriMark className="h-5 w-5" />
            </Link>
            {!collapsed && (
              <div className="min-w-0 leading-tight">
                <p className="text-sm font-medium tracking-tight">Yunori Mail</p>
                <p className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground">Console</p>
              </div>
            )}
          </div>

          <button
            type="button"
            onClick={() => setPaletteOpen(true)}
            aria-label="Search the console"
            aria-keyshortcuts="Meta+K Control+K"
            className={cn(
              "mb-4 flex items-center gap-2 rounded-md border border-border/60 bg-card/40 text-xs text-muted-foreground transition-colors hover:border-foreground/25 hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
              collapsed ? "mx-auto h-8 w-8 justify-center" : "mx-4 px-2.5 py-1.5",
            )}
          >
            <Search className="h-3.5 w-3.5" />
            {!collapsed && (
              <>
                Search…
                <kbd className="ml-auto rounded border border-border/60 px-1 text-[10px]">⌘K</kbd>
              </>
            )}
          </button>

          <SidebarNav groups={NAV} collapsed={collapsed} />

          <div className={cn("border-t border-border/60", collapsed ? "px-2 py-3" : "px-5 py-4")}>
            {!collapsed && (
              <>
                <p className="truncate text-xs text-foreground/90" title={admin.email}>
                  {admin.email}
                </p>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  {roleLabel(admin.role)}
                  {admin.totpEnabled ? " · 2FA on" : ""}
                </p>
              </>
            )}
            <button
              type="button"
              onClick={toggleCollapsed}
              aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
              aria-keyshortcuts="Meta+Backslash Control+Backslash"
              title={`${collapsed ? "Expand" : "Collapse"} sidebar (⌘\\)`}
              className={cn(
                "flex items-center gap-1.5 rounded-md text-[11px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                collapsed ? "mx-auto h-8 w-8 justify-center" : "mt-3",
              )}
            >
              {collapsed ? <PanelLeftOpen className="h-4 w-4" /> : <PanelLeftClose className="h-3.5 w-3.5" />}
              {!collapsed && "Collapse"}
            </button>
          </div>
        </aside>

        {/* Main column */}
        <div className="flex min-w-0 flex-1 flex-col">
          <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border/60 bg-background/80 px-3 backdrop-blur sm:px-5">
            <button type="button" onClick={() => setDrawerOpen(true)} aria-label="Open navigation" className={cn(iconButton, "lg:hidden")}>
              <Menu className="h-4 w-4" />
            </button>
            <Link to="/console" aria-label="Overview" className="lg:hidden">
              <YunoriMark className="h-5 w-5" />
            </Link>

            <nav aria-label="Breadcrumb" className="hidden min-w-0 items-center gap-1 text-xs sm:flex">
              <span className="text-muted-foreground">{here?.group.label ?? "Console"}</span>
              {here && here.item.label !== here.group.label && (
                <>
                  <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" aria-hidden />
                  {pathname.replace(/\/+$/, "") !== here.item.to ? (
                    <Link to={here.item.to} className="text-muted-foreground transition-colors hover:text-foreground">
                      {here.item.label}
                    </Link>
                  ) : (
                    <span className="truncate text-foreground/90" aria-current="page">
                      {here.item.label}
                    </span>
                  )}
                </>
              )}
              {here && pathname.replace(/\/+$/, "") !== here.item.to && (
                <>
                  <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground/60" aria-hidden />
                  <span className="truncate text-foreground/90" aria-current="page">
                    {crumb ?? "Detail"}
                  </span>
                </>
              )}
            </nav>

            <div className="ml-auto flex items-center gap-1">
              <span className="mr-1 hidden font-mono text-[11px] text-muted-foreground xl:inline" title="Server hostname">
                {session.server.hostname}
              </span>
              <HealthMenu health={health} hostname={session.server.hostname} />
              <button type="button" onClick={() => setPaletteOpen(true)} aria-label="Search" className={cn(iconButton, "lg:hidden")}>
                <Search className="h-4 w-4" />
              </button>
              <ThemeToggle />
              <AccountMenu />
            </div>
          </header>

          <main id="main" tabIndex={-1} className="min-w-0 flex-1 overflow-y-auto focus:outline-none" key={pathname}>
            <Suspense fallback={<PageFallback />}>
              <Outlet />
            </Suspense>
          </main>
        </div>

        {/* Mobile drawer */}
        <Dialog open={drawerOpen} onOpenChange={setDrawerOpen}>
          <DialogContent className="left-0 top-0 flex h-dvh max-w-[17rem] translate-x-0 translate-y-0 flex-col gap-0 rounded-none border-y-0 border-l-0 p-0 data-[state=closed]:zoom-out-100 data-[state=open]:zoom-in-100 sm:rounded-none">
            <DialogTitle className="sr-only">Console navigation</DialogTitle>
            <div className="flex items-center gap-2.5 px-5 pb-4 pt-5">
              <YunoriMark className="h-5 w-5" />
              <div className="min-w-0 leading-tight">
                <p className="text-sm font-medium tracking-tight">Yunori Mail</p>
                <p className="truncate font-mono text-[10px] text-muted-foreground">{session.server.hostname}</p>
              </div>
            </div>
            <SidebarNav groups={NAV} collapsed={false} onNavigate={() => setDrawerOpen(false)} />
            <div className="border-t border-border/60 px-5 py-4">
              <p className="truncate text-xs text-foreground/90">{admin.email}</p>
              <p className="mt-1 text-[11px] text-muted-foreground">{roleLabel(admin.role)}</p>
            </div>
          </DialogContent>
        </Dialog>

        <Palette open={paletteOpen} onOpenChange={setPaletteOpen} />
      </div>
    </ShellContext.Provider>
  );
}

function PageFallback() {
  return (
    <div className="mx-auto max-w-6xl px-5 py-8 sm:px-8 sm:py-10" aria-busy="true">
      <Skeleton className="mb-8 h-8 w-48" />
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {Array.from({ length: 4 }).map((_, i) => (
          <Skeleton key={i} className="h-24" />
        ))}
      </div>
    </div>
  );
}

export function roleLabel(role: string): string {
  return role === "owner" ? "Owner" : role === "admin" ? "Admin" : role === "viewer" ? "Viewer" : role;
}

// ── Sidebar ─────────────────────────────────────────────────────────────────

function SidebarNav({ groups, collapsed, onNavigate }: { groups: NavGroup[]; collapsed: boolean; onNavigate?: () => void }) {
  return (
    <nav className={cn("flex-1 overflow-y-auto pb-4", collapsed ? "px-2" : "px-3")} aria-label="Console sections">
      {groups.map((group, gi) => (
        <div key={group.label}>
          {gi > 0 && <div className={cn("my-3 border-t border-border/50", collapsed ? "mx-1" : "mx-2")} />}
          {!collapsed && (
            <p className="mb-1 px-2.5 text-[10px] font-medium uppercase tracking-[0.14em] text-muted-foreground/80">{group.label}</p>
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
          "group relative flex items-center gap-2.5 rounded-md py-1.5 text-[13px] transition-colors duration-150 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
          collapsed ? "justify-center px-0" : "px-2.5",
          isActive ? "bg-foreground/[0.05] text-foreground" : "text-muted-foreground hover:bg-accent/50 hover:text-foreground",
        )
      }
    >
      {({ isActive }) => (
        <>
          <span
            aria-hidden
            className={cn(
              "absolute left-0 top-1/2 h-3.5 w-[2px] -translate-y-1/2 rounded-full bg-torii transition-opacity",
              isActive ? "opacity-80" : "opacity-0",
            )}
          />
          <item.icon
            strokeWidth={1.5}
            className={cn("h-4 w-4 shrink-0 transition-colors", isActive ? "text-foreground/80" : "text-muted-foreground/80")}
          />
          {!collapsed && <span className="truncate">{item.label}</span>}
        </>
      )}
    </NavLink>
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
      <span className="mr-1 hidden items-center gap-1.5 rounded-full border border-border/60 px-2.5 py-1 text-[11px] text-muted-foreground md:inline-flex">
        <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-muted-foreground/40" />
        Checking…
      </span>
    );
  }
  const meta = VERDICT_META[health.verdict.level];
  const issues: Check[] = health.signals.filter((s) => s.status === "fail" || s.status === "warn");
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={`${hostname} health: ${meta.label}. ${health.verdict.sentence}`}
        className="mr-1 inline-flex items-center gap-1.5 rounded-full border border-border/60 px-2.5 py-1 text-[11px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
      >
        <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", toneDot(meta.tone))} />
        <span>{health.setup ? "Setting up" : meta.label}</span>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-80">
        <div className="px-2 py-1.5">
          <p className="text-xs font-medium">{meta.label}</p>
          <p className="mt-0.5 text-[11px] leading-relaxed text-muted-foreground">{health.verdict.sentence}</p>
        </div>
        <DropdownMenuSeparator />
        {health.setup && (
          <DropdownMenuItem onSelect={() => navigate("/console")} className="text-xs">
            <span aria-hidden className={cn("h-1.5 w-1.5 shrink-0 rounded-full", toneDot("warn"))} />
            Setup checklist
            <span className="ml-auto text-[11px] text-muted-foreground">In progress</span>
          </DropdownMenuItem>
        )}
        {issues.length === 0 ? (
          <p className="px-2 py-2 text-xs text-muted-foreground">Every signal passes. Quiet is good.</p>
        ) : (
          issues.map((s) => {
            const m = CHECK_META[s.status];
            return (
              <DropdownMenuItem key={s.id} onSelect={() => navigate(SIGNAL_TARGET[s.id] ?? "/console")} className="text-xs">
                <m.icon aria-hidden className={cn("h-3.5 w-3.5 shrink-0", toneText(m.tone))} />
                <span className="truncate">{s.label}</span>
                <span className={cn("ml-auto shrink-0 text-[11px]", toneText(m.tone))}>{m.label}</span>
              </DropdownMenuItem>
            );
          })
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => navigate("/console")} className="justify-center text-xs">
          Open overview
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function AccountMenu() {
  const { session, setSession } = useSession();
  const navigate = useNavigate();
  const admin = session.admin!;

  async function signOut() {
    try {
      await api.logout();
    } finally {
      setSession({ ...session, authenticated: false, admin: null });
      navigate("/console/login", { replace: true });
    }
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger aria-label={`Account: ${admin.email}`} className={cn(iconButton, "ml-0.5")}>
        <span
          aria-hidden
          className="grid h-6 w-6 place-items-center rounded-full border border-border bg-card text-[11px] font-medium uppercase text-foreground/80"
        >
          {admin.email.slice(0, 1)}
        </span>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-60">
        <div className="px-2 py-1.5">
          <p className="truncate text-xs font-medium">{admin.email}</p>
          <p className="mt-0.5 text-[11px] text-muted-foreground">
            {roleLabel(admin.role)} · two-step sign-in {admin.totpEnabled ? "on" : "off"}
          </p>
        </div>
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => navigate("/console/security")} className="text-xs">
          <UserRound className="h-3.5 w-3.5" aria-hidden />
          Admins & audit
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => void signOut()} className="text-xs">
          <LogOut className="h-3.5 w-3.5" aria-hidden />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
