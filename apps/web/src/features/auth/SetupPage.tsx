import { useNavigate } from "react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { useMe } from "@/lib/queries";
import type { PendingInvitation } from "@/lib/types";
import { Button, Field, Input, Select, useToast } from "@/components/ui";
import { Wordmark } from "@/app/Layout";
import { allTimezones, browserTimezone } from "@/lib/time";

const schema = z.object({
  name: z.string().trim().min(1, "Name your organization").max(120, "Use at most 120 characters"),
  timezone: z.string().min(1),
});

export function SetupPage() {
  const { data: me } = useMe();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const toast = useToast();
  const invitations = useQuery({ queryKey: ["my-invitations"], queryFn: () => api.get<PendingInvitation[]>("/me/invitations") });
  const form = useForm<z.infer<typeof schema>>({ resolver: zodResolver(schema), defaultValues: { name: "", timezone: browserTimezone() } });

  const create = useMutation({
    mutationFn: (v: z.infer<typeof schema>) => api.post("/organizations", v),
    onSuccess: async () => {
      await qc.invalidateQueries();
      toast("Organization created");
      navigate("/");
    },
  });
  const accept = useMutation({
    mutationFn: (id: string) => api.post("/invitations/accept", { id }),
    onSuccess: async () => {
      await qc.invalidateQueries();
      navigate("/");
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });

  return (
    <div className="min-h-dvh px-4 py-8 md:px-12 md:py-12">
      <Wordmark />
      <div className="mt-16 grid max-w-5xl gap-10 lg:grid-cols-2">
        <div className="rise">
          <h1 className="text-title font-semibold">Welcome, {me?.user.display_name}</h1>
          <p className="mt-4 text-lg">
            Everything in OncallKit belongs to an organization: its people, teams, schedules and Discord channels. Join one you were invited to, or start
            your own.
          </p>
          {(invitations.data?.length ?? 0) > 0 && (
            <div className="mt-8 flex flex-col gap-2">
              {invitations.data!.map((inv) => (
                <div key={inv.id} className="flex items-center justify-between gap-4 bg-signal-soft px-5 py-4">
                  <span>
                    <strong className="font-semibold">{inv.organization_name}</strong> invited you as {inv.role}
                  </span>
                  <Button variant="primary" size="sm" onClick={() => accept.mutate(inv.id)} disabled={accept.isPending}>
                    Join
                  </Button>
                </div>
              ))}
            </div>
          )}
        </div>
        {me?.can_create_organization ? (
          <form onSubmit={form.handleSubmit((v) => create.mutate(v))} className="rise rise-2 flex flex-col gap-5 bg-surface p-6 md:p-8" noValidate>
            <h2 className="text-2xl font-semibold tracking-[-0.02em]">Create an organization</h2>
            <Field label="Name" htmlFor="org-name" error={form.formState.errors.name?.message}>
              <Input id="org-name" placeholder="Acme Platform" aria-invalid={!!form.formState.errors.name} {...form.register("name")} />
            </Field>
            <Field label="Default timezone" htmlFor="org-tz" hint="New schedules start in this timezone. Each schedule can use its own.">
              <Select id="org-tz" {...form.register("timezone")}>
                {allTimezones().map((tz) => (
                  <option key={tz}>{tz}</option>
                ))}
              </Select>
            </Field>
            {create.error && <p role="alert" className="text-danger">{errorMessage(create.error)}</p>}
            <Button type="submit" variant="primary" disabled={create.isPending}>
              Create organization
            </Button>
            <p className="text-sm text-ink-soft">You become its owner. You can invite the rest of your team next.</p>
          </form>
        ) : (
          <div className="rise rise-2 bg-surface-2 p-6 md:p-8">
            <h2 className="text-2xl font-semibold tracking-[-0.02em]">Waiting for an invitation</h2>
            <p className="mt-3 text-lg">
              On this server only administrators create organizations. Ask one to invite <strong>{me?.user.email}</strong>; the invitation will appear here.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
