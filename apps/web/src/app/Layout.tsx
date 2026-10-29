import { useState, type ReactNode } from "react";
import { NavLink, Outlet, useNavigate } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import { Bell, CalendarClock, History, LayoutGrid, LogOut, Menu, Moon, Plug, ScrollText, Settings, Sun, SunMoon, Users, UsersRound, X } from "lucide-react";
import { api } from "@/lib/api";
import { useCan, useMe } from "@/lib/queries";
import type { Permission } from "@/lib/types";
import { cx } from "@/components/ui";
import { useTheme } from "./theme";

const NAV: { to: string; label: string; icon: typeof LayoutGrid; permission?: Permission }[] = [
  { to: "/", label: "Overview", icon: LayoutGrid },
  { to: "/schedules", label: "Schedules", icon: CalendarClock },
  { to: "/teams", label: "Teams", icon: UsersRound },
  { to: "/users", label: "Users", icon: Users },
  { to: "/integrations", label: "Integrations", icon: Plug, permission: "view_integrations" },
  { to: "/notifications", label: "Notification history", icon: Bell, permission: "view_integrations" },
  { to: "/audit", label: "Audit logs", icon: ScrollText, permission: "view_audit_logs" },
  { to: "/settings", label: "Settings", icon: Settings },
];

export function Wordmark({ onSignal = false }: { onSignal?: boolean }) {
  return (
    <span className="inline-flex items-center gap-2.5 text-[19px] font-semibold tracking-[-0.02em]">
      <span aria-hidden className={cx("inline-flex size-7 items-center justify-center", onSignal ? "bg-signal-ink" : "bg-signal")}>
        <svg viewBox="0 0 32 32" className="size-7">
          <path d="M9 23V9h4v10h10v4z" fill={onSignal ? "#f2c200" : "#141413"} />
        </svg>
      </span>
      OncallKit
    </span>
  );
}

function ThemeToggle() {
  const [theme, setTheme] = useTheme();
  const next = theme === "system" ? "light" : theme === "light" ? "dark" : "system";
  const Icon = theme === "system" ? SunMoon : theme === "light" ? Sun : Moon;
  return (
    <button type="button" onClick={() => setTheme(next)} className="flex w-full items-center gap-3 px-3 py-2 text-base hover:bg-surface-2" aria-label={`Theme: ${theme}. Switch to ${next}.`}>
      <Icon className="size-4" aria-hidden /> {theme === "system" ? "System theme" : theme === "light" ? "Light theme" : "Dark theme"}
    </button>
  );
}

function Sidebar({ onNavigate }: { onNavigate?: () => void }) {
  const { data: me } = useMe();
  const can = useCan();
  const qc = useQueryClient();
  const navigate = useNavigate();

  async function switchOrg(id: string) {
    await api.post(`/organizations/${id}/activate`);
    await qc.invalidateQueries();
    navigate("/");
    onNavigate?.();
  }

  async function signOut() {
    await api.post("/auth/logout").catch(() => {});
    qc.clear();
    navigate("/login");
  }

  return (
    <div className="flex h-full flex-col gap-8 bg-surface px-4 py-6">
      <div className="px-3">
        <Wordmark />
      </div>
      <nav aria-label="Main" className="flex flex-col gap-0.5">
        {NAV.filter((n) => !n.permission || can(n.permission)).map((n) => (
          <NavLink
            key={n.to}
            to={n.to}
            end={n.to === "/"}
            onClick={onNavigate}
            className={({ isActive }) =>
              cx("flex items-center gap-3 px-3 py-2.5 text-base transition-colors", isActive ? "bg-signal font-semibold text-signal-ink" : "hover:bg-surface-2")
            }
          >
            <n.icon className="size-[18px]" aria-hidden />
            {n.label}
          </NavLink>
        ))}
      </nav>
      <div className="mt-auto flex flex-col gap-1">
        {me && me.memberships.length > 1 && (
          <label className="mb-3 flex flex-col gap-1 px-3 text-sm text-ink-soft">
            Organization
            <select
              className="h-10 w-full bg-surface-2 px-2 text-base text-ink"
              value={me.active_organization_id ?? ""}
              onChange={(e) => switchOrg(e.target.value)}
            >
              {me.memberships.map((m) => (
                <option key={m.organization_id} value={m.organization_id}>
                  {m.name}
                </option>
              ))}
            </select>
          </label>
        )}
        <NavLink to="/profile" onClick={onNavigate} className={({ isActive }) => cx("flex items-center gap-3 px-3 py-2 text-base hover:bg-surface-2", isActive && "bg-surface-2 font-semibold")}>
          <History className="size-4" aria-hidden />
          <span className="truncate">{me?.user.display_name ?? "Profile"}</span>
        </NavLink>
        <ThemeToggle />
        <button type="button" onClick={signOut} className="flex items-center gap-3 px-3 py-2 text-base hover:bg-surface-2">
          <LogOut className="size-4" aria-hidden /> Sign out
        </button>
      </div>
    </div>
  );
}

export function AppLayout({ children }: { children?: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="min-h-dvh lg:grid lg:grid-cols-[260px_1fr]">
      <aside className="hidden lg:sticky lg:top-0 lg:block lg:h-dvh">
        <Sidebar />
      </aside>
      <div className="flex items-center justify-between bg-surface px-4 py-3 lg:hidden">
        <Wordmark />
        <button type="button" className="p-2 hover:bg-surface-2" onClick={() => setOpen(true)} aria-label="Open navigation">
          <Menu className="size-6" />
        </button>
      </div>
      {open && (
        <div className="fixed inset-0 z-40 lg:hidden" role="dialog" aria-modal="true" aria-label="Navigation">
          <div className="absolute inset-0 bg-black/40" onClick={() => setOpen(false)} />
          <div className="absolute inset-y-0 left-0 w-[min(300px,85vw)] shadow-2xl" /* taste-skill-ignore: floating drawer */>
            <button type="button" className="absolute right-3 top-5 z-10 p-2 hover:bg-surface-2" onClick={() => setOpen(false)} aria-label="Close navigation">
              <X className="size-5" />
            </button>
            <Sidebar onNavigate={() => setOpen(false)} />
          </div>
        </div>
      )}
      <main id="main" className="min-w-0 px-4 py-8 md:px-10 md:py-12 xl:px-14">
        {children ?? <Outlet />}
      </main>
    </div>
  );
}
