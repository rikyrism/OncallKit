import { useQuery, type UseQueryOptions } from "@tanstack/react-query";
import { useMemo } from "react";
import { api, setCsrfToken } from "./api";
import type { Integration, Me, Member, NotificationHealth, Overview, Permission, Schedule, Team } from "./types";

export const keys = {
  me: ["me"] as const,
  members: ["members"] as const,
  teams: ["teams"] as const,
  schedules: ["schedules"] as const,
  schedule: (id: string) => ["schedules", id] as const,
  overview: ["overview"] as const,
  integrations: ["integrations"] as const,
  health: ["notification-health"] as const,
};

export function useMe(options?: Partial<UseQueryOptions<Me | null>>) {
  return useQuery<Me | null>({
    queryKey: keys.me,
    queryFn: async () => {
      try {
        const me = await api.get<Me>("/me");
        setCsrfToken(me.csrf_token);
        return me;
      } catch (err) {
        if ((err as { status?: number }).status === 401) {
          setCsrfToken(null);
          return null;
        }
        throw err;
      }
    },
    staleTime: 60_000,
    ...options,
  });
}

/** Permission check against the signed-in user's role. UI only; the API enforces the same rules. */
export function useCan() {
  const { data } = useMe();
  return (p: Permission) => !!data?.permissions.includes(p);
}

export function useMembers() {
  return useQuery({ queryKey: keys.members, queryFn: () => api.get<Member[]>("/members") });
}

export function useMemberNames() {
  const { data } = useMembers();
  return useMemo(() => {
    const map = new Map<string, string>();
    for (const m of data ?? []) map.set(m.user_id, m.display_name);
    return (id: string | null | undefined) => (id ? (map.get(id) ?? "Former member") : "Nobody");
  }, [data]);
}

export function useTeams() {
  return useQuery({ queryKey: keys.teams, queryFn: () => api.get<Team[]>("/teams") });
}

export function useSchedules() {
  return useQuery({ queryKey: keys.schedules, queryFn: () => api.get<Schedule[]>("/schedules"), refetchInterval: 60_000 });
}

export function useOverview() {
  return useQuery({ queryKey: keys.overview, queryFn: () => api.get<Overview>("/oncall/overview"), refetchInterval: 60_000 });
}

export function useIntegrations(enabled = true) {
  return useQuery({ queryKey: keys.integrations, queryFn: () => api.get<Integration[]>("/integrations"), enabled });
}

export function useNotificationHealth() {
  return useQuery({ queryKey: keys.health, queryFn: () => api.get<NotificationHealth>("/notifications/health"), refetchInterval: 30_000 });
}
