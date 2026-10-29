import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { keys, useMe } from "@/lib/queries";
import type { SessionInfo } from "@/lib/types";
import { Button, Field, Input, Loading, PageHeader, Panel, Tag, useToast } from "@/components/ui";
import { browserTimezone, formatDateTime, relative } from "@/lib/time";
import { CalendarFeedsPanel } from "./CalendarFeeds";

export function ProfilePage() {
  const { data: me } = useMe();
  const qc = useQueryClient();
  const toast = useToast();
  const sessions = useQuery({ queryKey: ["sessions"], queryFn: () => api.get<SessionInfo[]>("/me/sessions") });
  const [form, setForm] = useState<{ display_name: string; discord_user_id: string } | null>(null);
  const values = form ?? (me ? { display_name: me.user.display_name, discord_user_id: me.user.discord_user_id ?? "" } : null);
  const save = useMutation({
    mutationFn: () => api.patch("/me", values),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.me });
      await qc.invalidateQueries({ queryKey: keys.members });
      setForm(null);
      toast("Profile saved");
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });
  const revoke = useMutation({
    mutationFn: (id: string) => api.delete(`/me/sessions/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["sessions"] }),
  });
  if (!me || !values) return <Loading />;
  const tz = browserTimezone();
  return (
    <div className="flex flex-col gap-6 md:gap-8">
      <PageHeader title="Your profile" lead={`Signed in as ${me.user.email}.`} />
      <Panel title="Details">
        <form
          className="flex max-w-lg flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate();
          }}
        >
          <Field label="Display name" htmlFor="p-name">
            <Input id="p-name" required maxLength={120} value={values.display_name} onChange={(e) => setForm({ ...values, display_name: e.target.value })} />
          </Field>
          <Field
            label="Discord user id"
            htmlFor="p-discord"
            hint="Optional. Lets integrations @mention you when you go on call. In Discord: Settings, Advanced, Developer Mode, then right-click your name and Copy User ID."
          >
            <Input id="p-discord" inputMode="numeric" pattern="[0-9]{5,25}" value={values.discord_user_id} onChange={(e) => setForm({ ...values, discord_user_id: e.target.value })} />
          </Field>
          <div>
            <Button type="submit" variant="primary" disabled={!form || save.isPending}>
              Save
            </Button>
          </div>
        </form>
      </Panel>
      <CalendarFeedsPanel />
      <Panel title="Sessions">
        {sessions.isLoading && <Loading />}
        <ul className="flex flex-col gap-4">
          {(sessions.data ?? []).map((s) => (
            <li key={s.id} className="flex flex-wrap items-start justify-between gap-3">
              <div>
                <p className="font-medium">
                  {s.user_agent ? s.user_agent.slice(0, 80) : "Unknown browser"} {s.current && <Tag tone="signal">This browser</Tag>}
                </p>
                <p className="text-sm text-ink-soft">
                  Signed in {formatDateTime(s.created_at, tz)} · last active {relative(s.last_seen_at)} · expires {formatDateTime(s.expires_at, tz)}
                  {s.ip_address && ` · ${s.ip_address}`}
                </p>
              </div>
              {!s.current && (
                <Button size="sm" onClick={() => revoke.mutate(s.id)}>
                  Sign out
                </Button>
              )}
            </li>
          ))}
        </ul>
      </Panel>
    </div>
  );
}
