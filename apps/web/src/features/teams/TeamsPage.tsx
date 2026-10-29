import { useState } from "react";
import { Link } from "react-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { keys, useCan, useMemberNames, useOverview, useTeams } from "@/lib/queries";
import { Button, Dialog, Empty, ErrorNote, Field, Input, Loading, PageHeader, Textarea, useToast } from "@/components/ui";

export function TeamDialog({ open, onClose, team }: { open: boolean; onClose: () => void; team?: { id: string; name: string; description: string } }) {
  const [name, setName] = useState(team?.name ?? "");
  const [description, setDescription] = useState(team?.description ?? "");
  const qc = useQueryClient();
  const toast = useToast();
  const save = useMutation({
    mutationFn: () => (team ? api.patch(`/teams/${team.id}`, { name, description }) : api.post("/teams", { name, description })),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.teams });
      toast(team ? "Team saved" : "Team created");
      onClose();
    },
  });
  return (
    <Dialog open={open} onClose={onClose} title={team ? "Edit team" : "New team"}>
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <Field label="Name" htmlFor="team-name">
          <Input id="team-name" required maxLength={120} value={name} onChange={(e) => setName(e.target.value)} placeholder="DevOps" />
        </Field>
        <Field label="Description" htmlFor="team-desc" hint="Optional.">
          <Textarea id="team-desc" rows={2} value={description} onChange={(e) => setDescription(e.target.value)} />
        </Field>
        {save.error && <p role="alert" className="text-danger">{errorMessage(save.error)}</p>}
        <div className="flex justify-end gap-2">
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={save.isPending || !name.trim()}>
            {team ? "Save" : "Create team"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function TeamsPage() {
  const { data, isLoading, error, refetch } = useTeams();
  const { data: overview } = useOverview();
  const name = useMemberNames();
  const can = useCan();
  const [open, setOpen] = useState(false);
  return (
    <div>
      <PageHeader title="Teams" lead="Group people and schedules, and send each team's notifications to its own Discord channel." actions={can("manage_teams") && <Button variant="primary" onClick={() => setOpen(true)}>Add team</Button>} />
      {isLoading && <Loading />}
      {error && <ErrorNote error={error} retry={() => refetch()} />}
      {data?.length === 0 && (
        <Empty title="No teams yet" action={can("manage_teams") ? <Button variant="primary" onClick={() => setOpen(true)}>Add team</Button> : undefined}>
          Teams are optional. They help when several groups each run their own rotation.
        </Empty>
      )}
      {data && data.length > 0 && (
        <ul className="flex flex-col gap-2">
          {data.map((t) => {
            const onCall = (overview?.schedules ?? []).filter((s) => s.team_id === t.id && s.on_call.primary);
            return (
              <li key={t.id}>
                <Link to={`/teams/${t.id}`} className="grid gap-4 bg-surface p-5 transition-colors hover:bg-surface-2 md:grid-cols-[1.2fr_1fr_1fr] md:items-center md:p-6">
                  <span>
                    <span className="block text-2xl font-semibold tracking-[-0.02em]">{t.name}</span>
                    {t.description && <span className="block text-base text-ink-soft">{t.description}</span>}
                  </span>
                  <span className="text-base">
                    {t.member_ids.length} {t.member_ids.length === 1 ? "person" : "people"} · {t.active_schedule_count} active {t.active_schedule_count === 1 ? "schedule" : "schedules"}
                  </span>
                  <span className="text-base">
                    On call: <strong className="font-semibold">{onCall.length ? Array.from(new Set(onCall.map((s) => name(s.on_call.primary!.user_id)))).join(", ") : "nobody"}</strong>
                  </span>
                </Link>
              </li>
            );
          })}
        </ul>
      )}
      {open && <TeamDialog open={open} onClose={() => setOpen(false)} />}
    </div>
  );
}
