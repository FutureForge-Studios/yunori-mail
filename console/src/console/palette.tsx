import { useEffect, useState, type ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { Command } from "cmdk";
import { Globe, Inbox, LogOut, Moon, Plus, Search, Stethoscope } from "lucide-react";
import { Dialog, DialogContent, DialogTitle } from "@/design/vendor/ui/dialog";
import { api } from "@/api/client";
import type { Domain, Mailbox } from "@/api/types";
import { useTheme } from "@/lib/theme";
import { NAV } from "./nav";
import { useSession } from "./session";

/**
 * ⌘K — jump to a section, run a common action, or open a domain/mailbox.
 * Same surface as the client's admin palette (cmdk in a dialog).
 */
export function Palette({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const navigate = useNavigate();
  const { resolvedTheme, setTheme } = useTheme();
  const { session, setSession } = useSession();
  const [domains, setDomains] = useState<Domain[]>([]);
  const [mailboxes, setMailboxes] = useState<Mailbox[]>([]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    Promise.all([api.domains(), api.mailboxes()])
      .then(([d, m]) => {
        if (cancelled) return;
        setDomains(d);
        setMailboxes(m);
      })
      .catch(() => {
        /* sections and actions still work */
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const go = (path: string) => {
    onOpenChange(false);
    navigate(path);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl gap-0 overflow-hidden p-0">
        <DialogTitle className="sr-only">Search the console</DialogTitle>
        <Command label="Search the console" className="bg-card">
          <div className="flex items-center gap-2 border-b border-border px-3">
            <Search className="h-4 w-4 text-muted-foreground" aria-hidden />
            <Command.Input
              placeholder="Jump to a section, domain or mailbox…"
              className="flex h-11 w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
            />
          </div>
          <Command.List className="max-h-80 overflow-y-auto p-2">
            <Command.Empty className="p-4 text-center text-sm text-muted-foreground">Nothing found.</Command.Empty>

            {NAV.map((g) => (
              <Command.Group key={g.label} heading={g.label} className="px-1 text-xs text-muted-foreground">
                {g.items.map((i) => (
                  <Item key={i.to} value={`${g.label} ${i.label} ${(i.keywords ?? []).join(" ")}`} onSelect={() => go(i.to)} icon={<i.icon className="h-4 w-4" />}>
                    {i.label}
                  </Item>
                ))}
              </Command.Group>
            ))}

            <Command.Group heading="Actions" className="px-1 text-xs text-muted-foreground">
              <Item value="new mailbox create user" onSelect={() => go("/console/mailboxes?new=1")} icon={<Plus className="h-4 w-4" />}>
                New mailbox
              </Item>
              <Item value="add domain create" onSelect={() => go("/console/domains?new=1")} icon={<Plus className="h-4 w-4" />}>
                Add domain
              </Item>
              <Item value="new alias forward" onSelect={() => go("/console/aliases?new=1")} icon={<Plus className="h-4 w-4" />}>
                New alias
              </Item>
              <Item value="run preflight diagnostics" onSelect={() => go("/console/system?preflight=1")} icon={<Stethoscope className="h-4 w-4" />}>
                Run preflight
              </Item>
              <Item
                value="toggle theme dark light mode"
                onSelect={() => {
                  setTheme(resolvedTheme === "dark" ? "light" : "dark");
                  onOpenChange(false);
                }}
                icon={<Moon className="h-4 w-4" />}
              >
                Switch to {resolvedTheme === "dark" ? "light" : "dark"} theme
              </Item>
              <Item
                value="sign out log out"
                onSelect={() => {
                  onOpenChange(false);
                  void api.logout().finally(() => {
                    setSession({ ...session, authenticated: false, admin: null });
                    navigate("/console/login", { replace: true });
                  });
                }}
                icon={<LogOut className="h-4 w-4" />}
              >
                Sign out
              </Item>
            </Command.Group>

            {domains.length > 0 && (
              <Command.Group heading="Domains" className="px-1 text-xs text-muted-foreground">
                {domains.map((d) => (
                  <Item key={d.id} value={`domain ${d.name}`} onSelect={() => go(`/console/domains/${d.id}`)} icon={<Globe className="h-4 w-4" />}>
                    {d.name}
                    {d.isPrimary && <Hint>primary</Hint>}
                  </Item>
                ))}
              </Command.Group>
            )}
            {mailboxes.length > 0 && (
              <Command.Group heading="Mailboxes" className="px-1 text-xs text-muted-foreground">
                {mailboxes.map((m) => (
                  <Item
                    key={m.id}
                    value={`mailbox ${m.address} ${m.displayName ?? ""}`}
                    onSelect={() => go(`/console/mailboxes?q=${encodeURIComponent(m.address)}`)}
                    icon={<Inbox className="h-4 w-4" />}
                  >
                    <span className="truncate">{m.address}</span>
                    {m.displayName && <Hint>{m.displayName}</Hint>}
                  </Item>
                ))}
              </Command.Group>
            )}
          </Command.List>
        </Command>
      </DialogContent>
    </Dialog>
  );
}

function Item({ children, icon, onSelect, value }: { children: ReactNode; icon: ReactNode; onSelect: () => void; value?: string }) {
  return (
    <Command.Item
      value={value}
      onSelect={onSelect}
      className="flex cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-2 text-sm text-foreground/90 aria-selected:bg-accent aria-selected:text-foreground"
    >
      <span className="text-muted-foreground" aria-hidden>
        {icon}
      </span>
      <span className="flex min-w-0 flex-1 items-center gap-2">{children}</span>
    </Command.Item>
  );
}

function Hint({ children }: { children: ReactNode }) {
  return <span className="ml-auto shrink-0 text-[11px] text-muted-foreground">{children}</span>;
}
