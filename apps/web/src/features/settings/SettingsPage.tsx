import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { useCan, useMe, useMembers } from "@/lib/queries";
import type { Organization } from "@/lib/types";
import { Button, ConfirmDialog, Field, Input, Loading, PageHeader, Panel, Select, useToast } from "@/components/ui";
import { allTimezones } from "@/lib/time";

export function SettingsPage() {
  const { data: me } = useMe();
  const can = useCan();
  const members = useMembers();
  const qc = useQueryClient();
  const toast = useToast();
  const org = useQuery({ queryKey: ["org", me?.active_organization_id], queryFn: () => api.get<Organization>(`/organizations/${me!.active_organization_id}`), enabled: !!me?.active_organization_id });
  const [form, setForm] = useState<{ name: string; timezone: string } | null>(null);
  const [newOwner, setNewOwner] = useState("");
  const [confirm, setConfirm] = useState(false);
  const values = form ?? (org.data ? { name: org.data.name, timezone: org.data.timezone } : null);

  const save = useMutation({
    mutationFn: () => api.patch("/organizations/current", values),
    onSuccess: async () => {
      await qc.invalidateQueries();
      toast("Settings saved");
      setForm(null);
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });
  const transfer = useMutation({
    mutationFn: () => api.post("/organizations/current/transfer-ownership", { user_id: newOwner }),
    onSuccess: async () => {
      await qc.invalidateQueries();
      toast("Ownership transferred. You are now an admin.");
      setConfirm(false);
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });

  if (!values) return <Loading />;
  const editable = can("manage_organization");
  return (
    <div className="flex flex-col gap-6 md:gap-8">
      <PageHeader title="Settings" lead={editable ? "Organization details. Only the owner can change these." : "Only the organization owner can change these settings."} />
      <Panel title="Organization">
        <form
          className="flex max-w-lg flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate();
          }}
        >
          <Field label="Name" htmlFor="org-name">
            <Input id="org-name" disabled={!editable} value={values.name} onChange={(e) => setForm({ ...values, name: e.target.value })} />
          </Field>
          <Field label="Default timezone" htmlFor="org-tz" hint="Used as the starting timezone for new schedules.">
            <Select id="org-tz" disabled={!editable} value={values.timezone} onChange={(e) => setForm({ ...values, timezone: e.target.value })}>
              {allTimezones().map((tz) => (
                <option key={tz}>{tz}</option>
              ))}
            </Select>
          </Field>
          {editable && (
            <div>
              <Button type="submit" variant="primary" disabled={!form || save.isPending}>
                Save
              </Button>
            </div>
          )}
        </form>
      </Panel>
      {can("transfer_ownership") && (
        <Panel title="Transfer ownership">
          <p className="mb-4 max-w-xl">The new owner gets full control, and you become an admin. This cannot be undone by you afterwards.</p>
          <div className="flex max-w-lg flex-wrap items-end gap-3">
            <Field label="New owner" htmlFor="owner">
              <Select id="owner" className="w-64" value={newOwner} onChange={(e) => setNewOwner(e.target.value)}>
                <option value="">Choose a member…</option>
                {(members.data ?? [])
                  .filter((m) => m.user_id !== me?.user.user_id)
                  .map((m) => (
                    <option key={m.user_id} value={m.user_id}>
                      {m.display_name}
                    </option>
                  ))}
              </Select>
            </Field>
            <Button variant="danger" disabled={!newOwner} onClick={() => setConfirm(true)}>
              Transfer
            </Button>
          </div>
        </Panel>
      )}
      <ConfirmDialog open={confirm} title="Transfer ownership?" confirmLabel="Transfer ownership" onClose={() => setConfirm(false)} onConfirm={() => transfer.mutate()} busy={transfer.isPending}>
        You will become an admin of {values.name}.
      </ConfirmDialog>
    </div>
  );
}
