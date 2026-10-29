import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { keys, useCan, useMemberNames, useMembers, useSchedules } from "@/lib/queries";
import type { Team } from "@/lib/types";
import { Button, ConfirmDialog, ErrorNote, Loading, PageHeader, Panel, Person, Select, useToast } from "@/components/ui";
import { formatDateTime } from "@/lib/time";
import { TeamDialog } from "./TeamsPage";

export function TeamDetailPage() {
  const { id = "" } = useParams();
  const can = useCan();
  const name = useMemberNames();
  const members = useMembers();
  const schedules = useSchedules();
  const qc = useQueryClient();
  const toast = useToast();
  const navigate = useNavigate();
  const team = useQuery({ queryKey: [...keys.teams, id], queryFn: () => api.get<Team>(`/teams/${id}`) });
  const [edit, setEdit] = useState(false);
  const [del, setDel] = useState(false);

  const setMembers = useMutation({
    mutationFn: (member_ids: string[]) => api.patch(`/teams/${id}`, { member_ids }),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.teams });
      await qc.invalidateQueries({ queryKey: keys.members });
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });
  const remove = useMutation({
    mutationFn: () => api.delete(`/teams/${id}`),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.teams });
      toast("Team deleted");
      navigate("/teams");
    },
  });

  if (team.isLoading) return <Loading />;
  if (!team.data) return <ErrorNote error={team.error ?? new Error("Team not found")} />;
  const t = team.data;
  const teamSchedules = (schedules.data ?? []).filter((s) => s.team_id === id);
  const outside = (members.data ?? []).filter((m) => !t.member_ids.includes(m.user_id));
  const manage = can("manage_teams");

  return (
    <div className="flex flex-col gap-6 md:gap-8">
      <PageHeader
        title={t.name}
        lead={t.description || undefined}
        actions={
          manage && (
            <>
              <Button onClick={() => setEdit(true)}>Rename</Button>
              <Button variant="ghost" onClick={() => setDel(true)}>
                Delete
              </Button>
            </>
          )
        }
      />
      <div className="grid gap-6 md:gap-8 xl:grid-cols-2">
        <Panel title="People">
          {t.member_ids.length === 0 && <p className="mb-4">Nobody is in this team yet.</p>}
          <ul className="flex flex-col gap-2">
            {t.member_ids.map((uid) => (
              <li key={uid} className="flex items-center justify-between gap-4">
                <Person name={name(uid)} />
                {manage && (
                  <Button size="sm" variant="ghost" onClick={() => setMembers.mutate(t.member_ids.filter((x) => x !== uid))}>
                    Remove
                  </Button>
                )}
              </li>
            ))}
          </ul>
          {manage && outside.length > 0 && (
            <label className="mt-5 flex flex-col gap-1.5 text-base font-medium">
              Add a person
              <Select value="" onChange={(e) => e.target.value && setMembers.mutate([...t.member_ids, e.target.value])}>
                <option value="">Choose…</option>
                {outside.map((m) => (
                  <option key={m.user_id} value={m.user_id}>
                    {m.display_name}
                  </option>
                ))}
              </Select>
            </label>
          )}
        </Panel>
        <Panel title="Schedules and upcoming shifts">
          {teamSchedules.length === 0 ? (
            <p>No schedules belong to this team.</p>
          ) : (
            <ul className="flex flex-col gap-5">
              {teamSchedules.map((s) => (
                <li key={s.id}>
                  <Link to={`/schedules/${s.id}`} className="text-lg font-semibold underline-offset-4 hover:underline">
                    {s.name}
                  </Link>
                  <p className="text-base">
                    On call: <strong>{s.on_call.primary ? name(s.on_call.primary.user_id) : "nobody"}</strong>
                    {s.on_call.next_rotation_at && ` · next hand-over ${formatDateTime(s.on_call.next_rotation_at, s.timezone)}`}
                  </p>
                </li>
              ))}
            </ul>
          )}
        </Panel>
      </div>
      {edit && <TeamDialog open={edit} onClose={() => setEdit(false)} team={t} />}
      <ConfirmDialog open={del} title={`Delete ${t.name}?`} confirmLabel="Delete team" onClose={() => setDel(false)} onConfirm={() => remove.mutate()} busy={remove.isPending}>
        Its schedules stay, without a team. Team-scoped Discord integrations are deleted.
      </ConfirmDialog>
    </div>
  );
}
