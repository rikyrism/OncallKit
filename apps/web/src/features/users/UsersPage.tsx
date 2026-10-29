import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { keys, useCan, useMe, useMembers } from "@/lib/queries";
import type { Invitation, Role } from "@/lib/types";
import { Button, ConfirmDialog, Dialog, ErrorNote, Field, Input, Loading, PageHeader, Panel, Person, Select, Tag, useToast } from "@/components/ui";
import { formatDateTime, browserTimezone } from "@/lib/time";

const ROLE_HELP: Record<Role, string> = {
  owner: "Everything, including organization settings",
  admin: "People, teams, schedules and integrations",
  scheduler: "Create and change schedules",
  member: "View schedules and who is on call",
};

function assignable(myRole: Role | null | undefined): Role[] {
  if (myRole === "owner") return ["admin", "scheduler", "member"];
  if (myRole === "admin") return ["scheduler", "member"];
  return [];
}

function InviteDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { data: me } = useMe();
  const roles = assignable(me?.role);
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<Role>("member");
  const [link, setLink] = useState<string | null>(null);
  const qc = useQueryClient();
  const toast = useToast();
  const invite = useMutation({
    mutationFn: () => api.post<{ invite_url: string }>("/invitations", { email, role }),
    onSuccess: async (r) => {
      setLink(r.invite_url);
      await qc.invalidateQueries({ queryKey: ["invitations"] });
    },
  });
  return (
    <Dialog open={open} onClose={onClose} title="Invite someone">
      {link ? (
        <div className="flex flex-col gap-4">
          <p className="text-lg">
            Send this link to <strong>{email}</strong>. It works only for that Google account and expires in 7 days. It is shown once.
          </p>
          <Input readOnly value={link} onFocus={(e) => e.currentTarget.select()} aria-label="Invitation link" />
          <div className="flex justify-end gap-2">
            <Button
              variant="primary"
              onClick={async () => {
                await navigator.clipboard.writeText(link).catch(() => {});
                toast("Link copied");
              }}
            >
              Copy link
            </Button>
            <Button onClick={onClose}>Done</Button>
          </div>
        </div>
      ) : (
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            invite.mutate();
          }}
        >
          <Field label="Email" htmlFor="inv-email" hint="The address they sign in to Google with.">
            <Input id="inv-email" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} />
          </Field>
          <Field label="Role" htmlFor="inv-role" hint={ROLE_HELP[role]}>
            <Select id="inv-role" value={role} onChange={(e) => setRole(e.target.value as Role)}>
              {roles.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
            </Select>
          </Field>
          {invite.error && <p role="alert" className="text-danger">{errorMessage(invite.error)}</p>}
          <div className="flex justify-end gap-2">
            <Button onClick={onClose}>Cancel</Button>
            <Button type="submit" variant="primary" disabled={invite.isPending}>
              Create invitation
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

export function UsersPage() {
  const { data: me } = useMe();
  const can = useCan();
  const members = useMembers();
  const qc = useQueryClient();
  const toast = useToast();
  const invitations = useQuery({ queryKey: ["invitations"], queryFn: () => api.get<Invitation[]>("/invitations"), enabled: can("manage_members") });
  const [inviting, setInviting] = useState(false);
  const [removing, setRemoving] = useState<{ id: string; name: string } | null>(null);
  const roles = assignable(me?.role);

  const changeRole = useMutation({
    mutationFn: ({ id, role }: { id: string; role: Role }) => api.patch(`/members/${id}`, { role }),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.members });
      toast("Role changed");
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.delete(`/members/${id}`),
    onSuccess: async () => {
      await qc.invalidateQueries();
      toast("Member removed");
      setRemoving(null);
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });
  const revoke = useMutation({
    mutationFn: (id: string) => api.delete(`/invitations/${id}`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["invitations"] }),
  });

  const open = (invitations.data ?? []).filter((i) => !i.accepted_at && !i.revoked_at && new Date(i.expires_at) > new Date());

  return (
    <div className="flex flex-col gap-6 md:gap-8">
      <PageHeader title="Users" lead="Everyone in this organization and what they can do." actions={can("manage_members") && <Button variant="primary" onClick={() => setInviting(true)}>Invite member</Button>} />
      {members.isLoading && <Loading />}
      {members.error && <ErrorNote error={members.error} />}
      {members.data && (
        <div className="overflow-x-auto bg-surface">
          <table className="data-table">
            <thead>
              <tr>
                <th>Person</th>
                <th>Role</th>
                <th>Joined</th>
                <th>
                  <span className="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {members.data.map((m) => {
                const editable = can("manage_members") && m.role !== "owner" && m.user_id !== me?.user.user_id && roles.includes(m.role as Role);
                return (
                  <tr key={m.user_id}>
                    <td>
                      <Person name={m.display_name} />
                      <span className="mt-0.5 block pl-10 text-sm text-ink-soft">{m.email}</span>
                    </td>
                    <td>
                      {editable ? (
                        <Select aria-label={`Role for ${m.display_name}`} className="h-9 w-40" value={m.role} onChange={(e) => changeRole.mutate({ id: m.user_id, role: e.target.value as Role })}>
                          {roles.map((r) => (
                            <option key={r} value={r}>
                              {r}
                            </option>
                          ))}
                        </Select>
                      ) : (
                        <Tag tone={m.role === "owner" ? "signal" : "neutral"}>{m.role}</Tag>
                      )}
                    </td>
                    <td className="tabular">{formatDateTime(m.joined_at, browserTimezone())}</td>
                    <td className="text-right">
                      {editable && (
                        <Button size="sm" variant="ghost" onClick={() => setRemoving({ id: m.user_id, name: m.display_name })}>
                          Remove
                        </Button>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      {can("manage_members") && (
        <Panel title="Open invitations">
          {open.length === 0 ? (
            <p>No open invitations.</p>
          ) : (
            <ul className="flex flex-col gap-3">
              {open.map((i) => (
                <li key={i.id} className="flex flex-wrap items-center justify-between gap-3">
                  <span>
                    <strong className="font-semibold">{i.email}</strong> as {i.role} · expires {formatDateTime(i.expires_at, browserTimezone())}
                  </span>
                  <Button size="sm" variant="ghost" onClick={() => revoke.mutate(i.id)}>
                    Revoke
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </Panel>
      )}
      {inviting && <InviteDialog open={inviting} onClose={() => setInviting(false)} />}
      <ConfirmDialog open={!!removing} title={`Remove ${removing?.name}?`} confirmLabel="Remove member" onClose={() => setRemoving(null)} onConfirm={() => removing && remove.mutate(removing.id)} busy={remove.isPending}>
        They lose access immediately and are taken out of every rotation from now on. Their past shifts stay in the history.
      </ConfirmDialog>
    </div>
  );
}
