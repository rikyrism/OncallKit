import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router";
import { Controller, useForm, useWatch } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, ApiError, errorMessage } from "@/lib/api";
import { keys, useCan, useMe, useMemberNames, useMembers, useTeams } from "@/lib/queries";
import type { Organization, ScheduleDetail, Shift } from "@/lib/types";
import { Button, Checkbox, ErrorNote, Field, Input, Loading, PageHeader, Select, Textarea, useToast } from "@/components/ui";
import { allTimezones, browserTimezone, formatDateTime, toLocalInput, zoneLabel, durationText } from "@/lib/time";
import { ParticipantList } from "./ParticipantList";
import { Timeline } from "./Timeline";
import { REMINDER_PRESETS, scheduleSchema, toPayload, type ScheduleForm } from "./schema";

function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

/** Live preview of the rotation, computed by the same server-side engine that runs in production. */
export function RotationPreview({ values }: { values: ScheduleForm }) {
  const name = useMemberNames();
  const parsed = scheduleSchema.safeParse(values);
  const payload = parsed.success ? toPayload(parsed.data) : null;
  const key = useDebounced(payload ? JSON.stringify(payload) : null, 350);
  const preview = useQuery({
    queryKey: ["schedule-preview", key],
    queryFn: () => api.post<{ start_at: string; end_at: string | null; shifts: Shift[] }>("/schedules/preview", JSON.parse(key!)),
    enabled: !!key,
    placeholderData: (prev) => prev,
  });
  if (!payload) return <p className="text-base">Fill in the start, the rotation and at least one person to see who will be on call.</p>;
  if (preview.error) return <ErrorNote error={preview.error} />;
  if (!preview.data) return <Loading label="Working out the shifts" />;
  const shifts = preview.data.shifts;
  const primary = shifts.filter((s) => s.layer === "primary").slice(0, 8);
  const first = shifts[0];
  const last = shifts[Math.min(shifts.length - 1, 13)];
  return (
    <div className="flex flex-col gap-6">
      {first && last && (
        <Timeline shifts={shifts.filter((s) => new Date(s.start) < new Date(last.end))} from={new Date(first.start)} to={new Date(last.end)} timezone={values.timezone} name={name} />
      )}
      <ol className="flex flex-col gap-3">
        {primary.map((s) => {
          const sec = shifts.find((x) => x.layer === "secondary" && x.start <= s.start && x.end > s.start);
          return (
            <li key={s.start} className="grid grid-cols-[1fr_auto] gap-x-4">
              <span className="font-semibold">
                {name(s.user_id)}
                {sec && <span className="font-normal"> with {name(sec.user_id)}</span>}
              </span>
              <span className="text-sm text-ink-soft">{durationText(s.start, s.end)}</span>
              <span className="col-span-2 text-sm tabular">
                {formatDateTime(s.start, values.timezone)} to {formatDateTime(s.end, values.timezone)} {zoneLabel(values.timezone, new Date(s.start))}
              </span>
            </li>
          );
        })}
      </ol>
    </div>
  );
}

export function ScheduleFormPage() {
  const { id } = useParams();
  const editing = !!id;
  const can = useCan();
  const { data: me } = useMe();
  const members = useMembers();
  const teams = useTeams();
  const org = useQuery({ queryKey: ["org", me?.active_organization_id], queryFn: () => api.get<Organization>(`/organizations/${me!.active_organization_id}`), enabled: !!me?.active_organization_id });
  const existing = useQuery({ queryKey: keys.schedule(id ?? ""), queryFn: () => api.get<ScheduleDetail>(`/schedules/${id}`), enabled: editing });
  const qc = useQueryClient();
  const navigate = useNavigate();
  const toast = useToast();
  const [applyFrom, setApplyFrom] = useState<"now" | "next_rotation">("next_rotation");

  const defaults: ScheduleForm = useMemo(() => {
    const s = existing.data;
    if (s) {
      return {
        name: s.name,
        description: s.description,
        team_id: s.team_id ?? "",
        timezone: s.timezone,
        start_local: s.start_local,
        open_ended: !s.end_local,
        end_local: s.end_local ?? "",
        interval: s.rotation?.interval ?? 7,
        unit: s.rotation?.unit ?? "days",
        primary: s.rotation?.primary ?? [],
        secondary: s.rotation?.secondary ?? [],
        reminder_offsets_minutes: s.reminder_offsets_minutes,
        notify_on_boundary: s.notify_on_boundary,
      };
    }
    const tz = org.data?.timezone ?? browserTimezone();
    const start = new Date();
    start.setDate(start.getDate() + 1);
    return {
      name: "",
      description: "",
      team_id: "",
      timezone: tz,
      start_local: toLocalInput(start, tz).slice(0, 11) + "09:00",
      open_ended: true,
      end_local: "",
      interval: 7,
      unit: "days",
      primary: [],
      secondary: [],
      reminder_offsets_minutes: [1440, 60],
      notify_on_boundary: true,
    };
  }, [existing.data, org.data]);

  const form = useForm<ScheduleForm>({ resolver: zodResolver(scheduleSchema), values: defaults, resetOptions: { keepDirtyValues: true } });
  const values = useWatch({ control: form.control }) as ScheduleForm;
  const errors = form.formState.errors;
  const started = !!existing.data && new Date(existing.data.start_at) <= new Date();
  const rotationChanged =
    !!existing.data &&
    JSON.stringify([values.interval, values.unit, values.primary, values.secondary]) !==
      JSON.stringify([existing.data.rotation?.interval, existing.data.rotation?.unit, existing.data.rotation?.primary, existing.data.rotation?.secondary]);

  const save = useMutation({
    mutationFn: async (v: ScheduleForm) => {
      const payload = toPayload(v);
      if (!editing) return api.post<ScheduleDetail>("/schedules", payload);
      const { rotation, ...rest } = payload;
      const body: Record<string, unknown> = { ...rest };
      if (started) {
        delete body.timezone;
        delete body.start_local;
      }
      if (rotationChanged) {
        body.rotation = rotation;
        body.apply_from = applyFrom;
      }
      return api.patch<ScheduleDetail>(`/schedules/${id}`, body);
    },
    onSuccess: async (s) => {
      await qc.invalidateQueries({ queryKey: keys.schedules });
      await qc.invalidateQueries({ queryKey: keys.overview });
      toast(editing ? "Schedule saved" : "Schedule created");
      navigate(`/schedules/${s.id}`);
    },
    onError: (err) => {
      if (err instanceof ApiError && err.field) {
        const field = err.field.startsWith("rotation.") ? "primary" : err.field;
        form.setError(field as keyof ScheduleForm, { message: err.message });
      }
    },
  });

  if (!can("manage_schedules")) return <ErrorNote error={new Error("You need the scheduler role or higher to change schedules.")} />;
  if (editing && existing.isLoading) return <Loading />;
  if (members.isLoading) return <Loading />;

  const memberList = members.data ?? [];

  return (
    <div>
      <PageHeader
        title={editing ? `Edit ${existing.data?.name ?? "schedule"}` : "New schedule"}
        lead="Choose when the rotation starts and how often it hands over. The preview shows exactly who will be on call."
      />
      <form onSubmit={form.handleSubmit((v) => save.mutate(v))} noValidate className="grid gap-8 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
        <div className="flex flex-col gap-8">
          <section className="flex flex-col gap-5 bg-surface p-5 md:p-7">
            <h2 className="text-xl font-semibold">Basics</h2>
            <Field label="Name" htmlFor="name" error={errors.name?.message}>
              <Input id="name" placeholder="DevOps On-call" aria-invalid={!!errors.name} {...form.register("name")} />
            </Field>
            <Field label="Description" htmlFor="description" hint="Optional. What this rotation covers.">
              <Textarea id="description" rows={2} {...form.register("description")} />
            </Field>
            <Field label="Team" htmlFor="team">
              <Select id="team" {...form.register("team_id")}>
                <option value="">No team</option>
                {(teams.data ?? []).map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name}
                  </option>
                ))}
              </Select>
            </Field>
          </section>

          <section className="flex flex-col gap-5 bg-surface p-5 md:p-7">
            <h2 className="text-xl font-semibold">When</h2>
            {started && <p className="bg-surface-2 px-4 py-3 text-base">This schedule has started, so its start and timezone are fixed. Past shifts never change.</p>}
            <Field label="Timezone" htmlFor="tz" hint="Hand-over times are kept in this timezone, including across daylight-saving changes.">
              <Select id="tz" disabled={started} {...form.register("timezone")}>
                {allTimezones().map((tz) => (
                  <option key={tz}>{tz}</option>
                ))}
              </Select>
            </Field>
            <div className="grid gap-5 md:grid-cols-2">
              <Field label="Starts" htmlFor="start" error={errors.start_local?.message} hint="The first hand-over happens here.">
                <Input id="start" type="datetime-local" disabled={started} aria-invalid={!!errors.start_local} {...form.register("start_local")} />
              </Field>
              <Field label="Ends" htmlFor="end" error={errors.end_local?.message}>
                <Input id="end" type="datetime-local" disabled={values.open_ended} aria-invalid={!!errors.end_local} {...form.register("end_local")} />
              </Field>
            </div>
            <Checkbox label="Open-ended" hint="Keep rotating until you set an end." {...form.register("open_ended")} />
          </section>

          <section className="flex flex-col gap-5 bg-surface p-5 md:p-7">
            <h2 className="text-xl font-semibold">Rotation</h2>
            <div className="grid grid-cols-[8rem_1fr] gap-3">
              <Field label="Every" htmlFor="interval" error={errors.interval?.message}>
                <Input id="interval" type="number" min={1} aria-invalid={!!errors.interval} {...form.register("interval")} />
              </Field>
              <Field label="Unit" htmlFor="unit">
                <Select id="unit" {...form.register("unit")}>
                  <option value="hours">hours</option>
                  <option value="days">days</option>
                  <option value="weeks">weeks</option>
                </Select>
              </Field>
            </div>
            <Field label="Primary rotation" htmlFor="primary" error={errors.primary?.message} hint="People take turns in this order.">
              <Controller control={form.control} name="primary" render={({ field }) => <ParticipantList id="primary" value={field.value} onChange={field.onChange} members={memberList} invalid={!!errors.primary} />} />
            </Field>
            <Field label="Secondary rotation" htmlFor="secondary" hint="Optional backup, with its own order. Leave empty for no secondary.">
              <Controller control={form.control} name="secondary" render={({ field }) => <ParticipantList id="secondary" value={field.value} onChange={field.onChange} members={memberList} />} />
            </Field>
            {editing && started && rotationChanged && (
              <div className="bg-signal-soft p-4" role="note">
                <p className="font-semibold">This changes who is on call from now on.</p>
                <p className="mt-1 text-base">Past shifts stay as they were. Pending Discord reminders are recalculated.</p>
                <fieldset className="mt-3 flex flex-col gap-2 text-base">
                  <legend className="sr-only">When should the new rotation take over?</legend>
                  <label className="flex items-center gap-2">
                    <input type="radio" checked={applyFrom === "next_rotation"} onChange={() => setApplyFrom("next_rotation")} /> At the next hand-over (the current shift finishes as planned)
                  </label>
                  <label className="flex items-center gap-2">
                    <input type="radio" checked={applyFrom === "now"} onChange={() => setApplyFrom("now")} /> Immediately
                  </label>
                </fieldset>
              </div>
            )}
          </section>

          <section className="flex flex-col gap-4 bg-surface p-5 md:p-7">
            <h2 className="text-xl font-semibold">Notifications</h2>
            <Checkbox label="Announce each hand-over" hint="Sent to the Discord channels subscribed to hand-overs." {...form.register("notify_on_boundary")} />
            <Controller
              control={form.control}
              name="reminder_offsets_minutes"
              render={({ field }) => (
                <fieldset className="flex flex-col gap-2">
                  <legend className="mb-1 text-base font-medium">Remind the next person</legend>
                  {REMINDER_PRESETS.map((p) => (
                    <Checkbox
                      key={p.minutes}
                      label={p.label}
                      checked={field.value.includes(p.minutes)}
                      onChange={(e) => field.onChange(e.target.checked ? [...field.value, p.minutes].sort((a, b) => b - a) : field.value.filter((m) => m !== p.minutes))}
                    />
                  ))}
                </fieldset>
              )}
            />
          </section>

          {save.error && !(save.error instanceof ApiError && save.error.field) && <ErrorNote error={new Error(errorMessage(save.error))} />}
          <div className="flex gap-2">
            <Button type="submit" variant="primary" disabled={save.isPending}>
              {editing ? "Save schedule" : "Create schedule"}
            </Button>
            <Button onClick={() => navigate(-1)}>Cancel</Button>
          </div>
        </div>

        <aside className="xl:sticky xl:top-8 xl:self-start">
          <section className="bg-surface p-5 md:p-7" aria-live="polite">
            <h2 className="mb-5 text-xl font-semibold">Preview</h2>
            <RotationPreview values={values} />
          </section>
        </aside>
      </form>
    </div>
  );
}
