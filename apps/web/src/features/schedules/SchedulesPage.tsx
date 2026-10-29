import { Link, useSearchParams } from "react-router";
import { useCan, useMemberNames, useSchedules, useTeams } from "@/lib/queries";
import { Empty, ErrorNote, LinkButton, Loading, PageHeader, Select, Tag } from "@/components/ui";
import { formatDateTime, zoneLabel } from "@/lib/time";
import type { Schedule } from "@/lib/types";

export const STATE_LABEL: Record<Schedule["state"], { text: string; tone: "neutral" | "signal" | "ok" | "danger" }> = {
  active: { text: "Active", tone: "ok" },
  not_started: { text: "Starts later", tone: "neutral" },
  ended: { text: "Ended", tone: "neutral" },
  paused: { text: "Notifications paused", tone: "neutral" },
  archived: { text: "Archived", tone: "neutral" },
};

export function rotationText(s: { rotation: Schedule["rotation"] }): string {
  const r = s.rotation;
  if (!r) return "No rotation";
  const unit = r.interval === 1 ? r.unit.replace(/s$/, "") : r.unit;
  return r.interval === 1 ? `Every ${unit}` : `Every ${r.interval} ${unit}`;
}

export function SchedulesPage() {
  const { data, isLoading, error, refetch } = useSchedules();
  const { data: teams } = useTeams();
  const can = useCan();
  const name = useMemberNames();
  const [params, setParams] = useSearchParams();
  const team = params.get("team") ?? "";
  const list = (data ?? []).filter((s) => !team || s.team_id === team);
  const teamName = (id: string | null) => teams?.find((t) => t.id === id)?.name;

  return (
    <div>
      <PageHeader
        title="Schedules"
        lead="Each schedule rotates its people automatically. Who is on call is worked out from the schedule itself, so it is right even after a restart."
        actions={can("manage_schedules") && <LinkButton to="/schedules/new" variant="primary">Create schedule</LinkButton>}
      />
      {(teams?.length ?? 0) > 0 && (
        <div className="mb-6 max-w-xs">
          <label className="flex flex-col gap-1.5 text-base font-medium">
            Team
            <Select value={team} onChange={(e) => setParams(e.target.value ? { team: e.target.value } : {})}>
              <option value="">All teams</option>
              {teams!.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </Select>
          </label>
        </div>
      )}
      {isLoading && <Loading />}
      {error && <ErrorNote error={error} retry={() => refetch()} />}
      {data && list.length === 0 && (
        <Empty title={team ? "No schedules for this team" : "No schedules yet"} action={can("manage_schedules") ? <LinkButton to="/schedules/new" variant="primary">Create schedule</LinkButton> : undefined}>
          {can("manage_schedules") ? "Create a schedule to start rotating on-call duty." : "Ask an administrator or scheduler to create one."}
        </Empty>
      )}
      {list.length > 0 && (
        <div className="overflow-x-auto bg-surface">
          <table className="data-table">
            <thead>
              <tr>
                <th>Schedule</th>
                <th>On call now</th>
                <th>Rotation</th>
                <th>Next hand-over</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {list.map((s) => (
                <tr key={s.id}>
                  <td>
                    <Link to={`/schedules/${s.id}`} className="text-lg font-semibold underline-offset-4 hover:underline">
                      {s.name}
                    </Link>
                    <span className="block text-sm text-ink-soft">
                      {teamName(s.team_id) ?? "No team"} · {s.timezone}
                    </span>
                  </td>
                  <td>
                    <span className="block font-semibold">{s.on_call.primary ? name(s.on_call.primary.user_id) : "Nobody"}</span>
                    {s.on_call.secondary && <span className="block text-sm">Secondary: {name(s.on_call.secondary.user_id)}</span>}
                  </td>
                  <td>{rotationText(s)}</td>
                  <td className="tabular">
                    {s.on_call.next_rotation_at ? `${formatDateTime(s.on_call.next_rotation_at, s.timezone)} ${zoneLabel(s.timezone)}` : "—"}
                  </td>
                  <td>
                    <Tag tone={STATE_LABEL[s.state].tone}>{STATE_LABEL[s.state].text}</Tag>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
