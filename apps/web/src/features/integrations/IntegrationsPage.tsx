import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, ApiError, errorMessage } from "@/lib/api";
import { keys, useCan, useIntegrations, useTeams } from "@/lib/queries";
import type { Integration } from "@/lib/types";
import { Button, Checkbox, ConfirmDialog, Dialog, Empty, ErrorNote, Field, Input, Loading, PageHeader, Select, Tag, useToast } from "@/components/ui";
import { browserTimezone, formatDateTime } from "@/lib/time";

export const EVENT_LABELS: Record<string, string> = {
  rotation_changed: "Rotation hand-over",
  primary_changed: "New primary on call (including overrides)",
  secondary_changed: "New secondary on call (including overrides)",
  rotation_approaching: "Reminder before a shift",
  schedule_created: "Schedule created",
  schedule_updated: "Schedule updated",
  override_created: "Override created",
  override_canceled: "Override canceled",
  delivery_failed: "Another channel failed to deliver",
};

const DISCORD_URL = /^https:\/\/(?:ptb\.|canary\.)?discord(?:app)?\.com\/api\/(?:v\d+\/)?webhooks\/\d+\/[\w-]{20,}(?:\?thread_id=\d+)?$/;

export function validateWebhook(url: string): string | null {
  if (!url.trim()) return "Paste the webhook URL from Discord";
  return DISCORD_URL.test(url.trim()) ? null : "Use a Discord webhook URL: https://discord.com/api/webhooks/…";
}

function IntegrationDialog({ open, onClose, existing }: { open: boolean; onClose: () => void; existing?: Integration }) {
  const teams = useTeams();
  const events = useQuery({ queryKey: ["integration-events"], queryFn: () => api.get<string[]>("/integrations/events") });
  const qc = useQueryClient();
  const toast = useToast();
  const [form, setForm] = useState({
    name: existing?.name ?? "",
    webhook_url: "",
    team_id: existing?.team_id ?? "",
    events: existing?.events ?? ["rotation_changed", "rotation_approaching", "override_created", "override_canceled"],
    mention_users: existing?.mention_users ?? false,
    message_prefix: existing?.message_prefix ?? "",
    max_attempts: existing?.max_attempts ?? 6,
  });
  const urlError = existing && !form.webhook_url ? null : validateWebhook(form.webhook_url);
  const [touched, setTouched] = useState(false);
  const save = useMutation({
    mutationFn: () => {
      const body = { ...form, team_id: form.team_id || null, webhook_url: form.webhook_url || undefined, max_attempts: Number(form.max_attempts) };
      return existing ? api.patch(`/integrations/${existing.id}`, body) : api.post("/integrations/discord", body);
    },
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.integrations });
      toast(existing ? "Integration saved" : "Discord connected. Send a test message to check it.");
      onClose();
    },
  });
  const toggleEvent = (e: string, on: boolean) => setForm((f) => ({ ...f, events: on ? [...f.events, e] : f.events.filter((x) => x !== e) }));

  return (
    <Dialog open={open} onClose={onClose} title={existing ? `Edit ${existing.name}` : "Connect a Discord channel"} wide>
      <form
        className="grid gap-6 md:grid-cols-2"
        noValidate
        onSubmit={(e) => {
          e.preventDefault();
          setTouched(true);
          if (!urlError && form.events.length) save.mutate();
        }}
      >
        <div className="flex flex-col gap-4">
          <Field label="Name" htmlFor="int-name">
            <Input id="int-name" required value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder="#devops-oncall" />
          </Field>
          <Field
            label="Webhook URL"
            htmlFor="int-url"
            error={touched && urlError ? urlError : undefined}
            hint={existing ? `Saved: ${existing.webhook_hint}. Leave empty to keep it.` : "In Discord: channel settings, Integrations, Webhooks, New webhook, Copy URL."}
          >
            <Input
              id="int-url"
              type="url"
              autoComplete="off"
              spellCheck={false}
              value={form.webhook_url}
              aria-invalid={touched && !!urlError}
              onChange={(e) => setForm({ ...form, webhook_url: e.target.value })}
              placeholder={existing ? "••••••••" : "https://discord.com/api/webhooks/…"}
            />
          </Field>
          <Field label="Scope" htmlFor="int-team" hint="Team channels only hear about that team's schedules.">
            <Select id="int-team" value={form.team_id} onChange={(e) => setForm({ ...form, team_id: e.target.value })}>
              <option value="">Whole organization</option>
              {(teams.data ?? []).map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </Select>
          </Field>
          <Field label="Message prefix" htmlFor="int-prefix" hint="Optional text before every message.">
            <Input id="int-prefix" maxLength={200} value={form.message_prefix} onChange={(e) => setForm({ ...form, message_prefix: e.target.value })} />
          </Field>
          <Field label="Delivery attempts" htmlFor="int-attempts" hint="Retries back off from 30 seconds up to an hour.">
            <Input id="int-attempts" type="number" min={1} max={20} value={form.max_attempts} onChange={(e) => setForm({ ...form, max_attempts: Number(e.target.value) })} />
          </Field>
          <Checkbox
            label="Mention people"
            hint="Pings the person on call, if they added their Discord user id to their profile."
            checked={form.mention_users}
            onChange={(e) => setForm({ ...form, mention_users: e.target.checked })}
          />
        </div>
        <fieldset className="flex flex-col gap-3">
          <legend className="mb-2 text-base font-medium">Send these events</legend>
          {(events.data ?? Object.keys(EVENT_LABELS)).map((e) => (
            <Checkbox key={e} label={EVENT_LABELS[e] ?? e} checked={form.events.includes(e)} onChange={(ev) => toggleEvent(e, ev.target.checked)} />
          ))}
          {touched && form.events.length === 0 && <p role="alert" className="text-sm text-danger">Choose at least one event.</p>}
        </fieldset>
        {save.error && (
          <p role="alert" className="text-danger md:col-span-2">
            {errorMessage(save.error)}
            {save.error instanceof ApiError && save.error.field ? ` (${save.error.field})` : ""}
          </p>
        )}
        <div className="flex justify-end gap-2 md:col-span-2">
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={save.isPending}>
            {existing ? "Save" : "Connect"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function IntegrationsPage() {
  const can = useCan();
  const { data, isLoading, error, refetch } = useIntegrations(can("view_integrations"));
  const teams = useTeams();
  const qc = useQueryClient();
  const toast = useToast();
  const [editing, setEditing] = useState<Integration | "new" | null>(null);
  const [deleting, setDeleting] = useState<Integration | null>(null);
  const manage = can("manage_integrations");

  const test = useMutation({
    mutationFn: (i: Integration) => api.post<{ delivered: boolean; status_code: number | null; error: string | null }>(`/integrations/${i.id}/test`),
    onSuccess: async (r) => {
      await qc.invalidateQueries({ queryKey: keys.integrations });
      if (r.delivered) toast("Discord accepted the test message");
      else toast(`Discord did not accept it: ${r.error ?? "unknown error"}`, "danger");
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });
  const toggle = useMutation({
    mutationFn: (i: Integration) => api.patch(`/integrations/${i.id}`, { enabled: !i.enabled }),
    onSuccess: () => qc.invalidateQueries({ queryKey: keys.integrations }),
  });
  const remove = useMutation({
    mutationFn: (i: Integration) => api.delete(`/integrations/${i.id}`),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.integrations });
      setDeleting(null);
      toast("Integration deleted");
    },
  });

  if (!can("view_integrations")) return <ErrorNote error={new Error("Only administrators can see integrations.")} />;

  return (
    <div>
      <PageHeader
        title="Integrations"
        lead="Discord channels that hear about hand-overs, reminders and overrides. Webhook URLs are encrypted at rest and never shown again after saving."
        actions={manage && <Button variant="primary" onClick={() => setEditing("new")}>Connect Discord</Button>}
      />
      {isLoading && <Loading />}
      {error && <ErrorNote error={error} retry={() => refetch()} />}
      {data?.length === 0 && (
        <Empty title="No Discord channel connected" action={manage ? <Button variant="primary" onClick={() => setEditing("new")}>Connect Discord</Button> : undefined}>
          Create a webhook in the Discord channel where your team coordinates on-call, then paste its URL here. OncallKit posts each hand-over the moment it
          happens, and reminds the next person before their shift.
        </Empty>
      )}
      {data && data.length > 0 && (
        <ul className="flex flex-col gap-2">
          {data.map((i) => (
            <li key={i.id} className="grid gap-4 bg-surface p-5 md:grid-cols-[1.3fr_1fr_auto] md:items-center md:p-6">
              <div>
                <p className="text-2xl font-semibold tracking-[-0.02em]">{i.name}</p>
                <p className="text-base">
                  Discord · {teams.data?.find((t) => t.id === i.team_id)?.name ?? "Whole organization"} · {i.events.length} events
                </p>
                <p className="text-sm text-ink-soft">
                  {i.webhook_hint}
                  {!i.encrypted && " · stored unencrypted (set ENCRYPTION_KEY)"}
                </p>
              </div>
              <div className="flex flex-wrap items-center gap-2 text-base">
                {!i.enabled && <Tag>Disabled</Tag>}
                {i.last_delivery_status === "succeeded" && <Tag tone="ok">Last delivery succeeded</Tag>}
                {i.last_delivery_status === "failed" && <Tag tone="danger">Last delivery failed</Tag>}
                {i.last_delivery_at && <span className="text-sm">{formatDateTime(i.last_delivery_at, browserTimezone())}</span>}
                {!i.last_delivery_status && <span>No deliveries yet</span>}
              </div>
              {manage && (
                <div className="flex flex-wrap gap-2">
                  <Button size="sm" variant="primary" onClick={() => test.mutate(i)} disabled={test.isPending}>
                    Send test
                  </Button>
                  <Button size="sm" onClick={() => setEditing(i)}>
                    Edit
                  </Button>
                  <Button size="sm" onClick={() => toggle.mutate(i)}>
                    {i.enabled ? "Disable" : "Enable"}
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => setDeleting(i)}>
                    Delete
                  </Button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
      {editing && <IntegrationDialog open onClose={() => setEditing(null)} existing={editing === "new" ? undefined : editing} />}
      <ConfirmDialog open={!!deleting} title={`Delete ${deleting?.name}?`} confirmLabel="Delete integration" onClose={() => setDeleting(null)} onConfirm={() => deleting && remove.mutate(deleting)} busy={remove.isPending}>
        Pending deliveries to this channel are dropped along with its delivery history.
      </ConfirmDialog>
    </div>
  );
}
