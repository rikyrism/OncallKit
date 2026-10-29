import { useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { api, errorMessage, query } from "@/lib/api";
import { keys, useCan, useMemberNames, useMembers, useTeams } from "@/lib/queries";
import type { OnCallNow, Override, ScheduleDetail, Shift, Transition } from "@/lib/types";
import { Button, ConfirmDialog, Dialog, ErrorNote, Field, Input, LinkButton, Loading, Panel, Select, Tag, cx, useToast } from "@/components/ui";
import { browserTimezone, formatDateTime, formatDay, formatLong, formatTime, localDateKey, relative, toLocalInput, zoneLabel } from "@/lib/time";
import { Timeline, toneFor } from "./Timeline";
import { STATE_LABEL, rotationText } from "./SchedulesPage";

type View = "timeline" | "calendar" | "list";

function OnCallField({ s, displayTz }: { s: ScheduleDetail; displayTz: string }) {
  const name = useMemberNames();
  const p = s.on_call.primary;
  const sec = s.on_call.secondary;
  return (
    <section className="rise bg-signal px-5 py-6 text-signal-ink md:px-8 md:py-8" aria-label="On call now">
      {!p && <p className="text-3xl font-semibold tracking-[-0.02em]">{s.state === "not_started" ? `Starts ${formatLong(s.start_at, displayTz)}` : "Nobody is on call now"}</p>}
      {p && (
        <div className="grid gap-6 md:grid-cols-[minmax(0,1.6fr)_minmax(0,1fr)] md:items-end">
          <div>
            <p className="text-display font-semibold">{name(p.user_id)}</p>
            <p className="mt-3 text-lg">On call now as primary{p.source.kind === "override" ? ", covering by override" : ""}.</p>
          </div>
          <dl className="grid grid-cols-2 gap-x-6 gap-y-3 text-base">
            <div>
              <dt>Secondary</dt>
              <dd className="text-lg font-semibold">{sec ? name(sec.user_id) : "None"}</dd>
            </div>
            <div>
              <dt>Until</dt>
              <dd className="text-lg font-semibold tabular">
                {formatDay(p.end, displayTz)}, {formatTime(p.end, displayTz)}
              </dd>
            </div>
            <div className="col-span-2">
              <dt className="sr-only">Shift</dt>
              <dd>
                Shift began {formatDateTime(p.start, displayTz)} {zoneLabel(displayTz)}.
                {s.on_call.next_rotation_at && ` Next rotation ${relative(s.on_call.next_rotation_at)}.`}
              </dd>
            </div>
          </dl>
        </div>
      )}
    </section>
  );
}

/** The live tool: pick any moment and see who holds each layer then. */
function WhoIsOnCallAt({ s, displayTz }: { s: ScheduleDetail; displayTz: string }) {
  const name = useMemberNames();
  const [local, setLocal] = useState(() => toLocalInput(new Date(Date.now() + 7 * 86_400_000), s.timezone));
  const at = useMemo(() => localToUtc(local, s.timezone), [local, s.timezone]);
  const q = useQuery({
    queryKey: ["on-call-at", s.id, at],
    queryFn: () => api.get<OnCallNow>(`/schedules/${s.id}/on-call-now${query({ at })}`),
    enabled: !!at,
    placeholderData: (p) => p,
  });
  return (
    <Panel title="Who is on call at…">
      <Field label={`Moment (${s.timezone})`} htmlFor="who-at">
        <Input id="who-at" type="datetime-local" value={local} onChange={(e) => setLocal(e.target.value)} />
      </Field>
      {q.data && (
        <dl className="mt-5 grid grid-cols-2 gap-4">
          <div>
            <dt className="text-sm">Primary</dt>
            <dd className="text-2xl font-semibold tracking-[-0.02em]">{q.data.primary ? name(q.data.primary.user_id) : "Nobody"}</dd>
          </div>
          <div>
            <dt className="text-sm">Secondary</dt>
            <dd className="text-2xl font-semibold tracking-[-0.02em]">{q.data.secondary ? name(q.data.secondary.user_id) : "Nobody"}</dd>
          </div>
          {q.data.primary && (
            <p className="col-span-2 text-sm">
              Shift {formatDateTime(q.data.primary.start, displayTz)} to {formatDateTime(q.data.primary.end, displayTz)} {zoneLabel(displayTz)}
            </p>
          )}
        </dl>
      )}
    </Panel>
  );
}

/** Convert a wall-clock time in `tz` to a UTC ISO string (DST gaps resolve forward, like the server). */
export function localToUtc(local: string, tz: string): string | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(local);
  if (!m) return null;
  const [, y, mo, d, h, mi] = m.map(Number) as [number, number, number, number, number, number];
  let guess = Date.UTC(y, mo - 1, d, h, mi);
  for (let i = 0; i < 3; i++) {
    const shown = toLocalInput(new Date(guess), tz);
    const sm = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})$/.exec(shown)!.map(Number);
    const shownMs = Date.UTC(sm[1]!, sm[2]! - 1, sm[3]!, sm[4]!, sm[5]!);
    const diff = Date.UTC(y, mo - 1, d, h, mi) - shownMs;
    if (diff === 0) break;
    guess += diff;
  }
  return new Date(guess).toISOString().replace(/\.\d{3}Z$/, "Z");
}

function CalendarView({ s, displayTz }: { s: ScheduleDetail; displayTz: string }) {
  const name = useMemberNames();
  const [month, setMonth] = useState(() => {
    const k = localDateKey(new Date(), displayTz);
    return k.slice(0, 7);
  });
  const [y, m] = month.split("-").map(Number) as [number, number];
  const first = new Date(Date.UTC(y, m - 1, 1));
  const startOffset = (first.getUTCDay() + 6) % 7; // Monday first
  const daysInMonth = new Date(Date.UTC(y, m, 0)).getUTCDate();
  const from = new Date(Date.UTC(y, m - 1, 1) - 2 * 86_400_000).toISOString();
  const to = new Date(Date.UTC(y, m, 1) + 2 * 86_400_000).toISOString();
  const shifts = useQuery({ queryKey: ["assignments", s.id, from, to], queryFn: () => api.get<Shift[]>(`/schedules/${s.id}/assignments${query({ from, to })}`) });
  const order = useMemo(() => Array.from(new Set((shifts.data ?? []).map((x) => x.user_id))), [shifts.data]);
  const today = localDateKey(new Date(), displayTz);

  const byDay = useMemo(() => {
    const map = new Map<string, Shift[]>();
    for (const sh of shifts.data ?? []) {
      if (sh.layer !== "primary") continue;
      for (let t = new Date(sh.start).getTime(); t < new Date(sh.end).getTime(); t += 3_600_000) {
        const key = localDateKey(new Date(t), displayTz);
        const list = map.get(key) ?? [];
        if (!list.includes(sh)) list.push(sh);
        map.set(key, list);
      }
    }
    return map;
  }, [shifts.data, displayTz]);

  const shift = (d: number) => {
    const dt = new Date(Date.UTC(y, m - 1 + d, 1));
    setMonth(`${dt.getUTCFullYear()}-${String(dt.getUTCMonth() + 1).padStart(2, "0")}`);
  };

  return (
    <div>
      <div className="mb-4 flex items-center gap-2">
        <Button size="sm" variant="ghost" onClick={() => shift(-1)} aria-label="Previous month">
          <ChevronLeft className="size-5" />
        </Button>
        <h3 className="min-w-44 text-center text-lg font-semibold">{new Intl.DateTimeFormat("en-GB", { month: "long", year: "numeric", timeZone: "UTC" }).format(first)}</h3>
        <Button size="sm" variant="ghost" onClick={() => shift(1)} aria-label="Next month">
          <ChevronRight className="size-5" />
        </Button>
      </div>
      <div className="grid grid-cols-7 gap-1 text-sm" role="grid" aria-label="Primary on-call calendar">
        {["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map((d) => (
          <div key={d} className="px-2 pb-1 font-medium" role="columnheader">
            {d}
          </div>
        ))}
        {Array.from({ length: startOffset }).map((_, i) => (
          <div key={`pad-${i}`} />
        ))}
        {Array.from({ length: daysInMonth }).map((_, i) => {
          const key = `${month}-${String(i + 1).padStart(2, "0")}`;
          const list = byDay.get(key) ?? [];
          return (
            <div key={key} role="gridcell" className={cx("min-h-20 bg-surface-2 p-1.5 md:min-h-24", key === today && "outline-2 -outline-offset-2 outline-[var(--ink)]")}>
              <div className="mb-1 font-semibold tabular">{i + 1}</div>
              <div className="flex flex-col gap-0.5">
                {list.slice(0, 3).map((sh) => {
                  const startsToday = localDateKey(sh.start, displayTz) === key;
                  return (
                    <span key={sh.start} className={cx("truncate px-1.5 py-0.5 text-xs font-medium", toneFor(sh.user_id, order))} title={`${name(sh.user_id)} ${formatDateTime(sh.start, displayTz)} to ${formatDateTime(sh.end, displayTz)}`}>
                      {startsToday ? `${formatTime(sh.start, displayTz)} ` : ""}
                      {name(sh.user_id)}
                    </span>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function UpcomingList({ s, displayTz }: { s: ScheduleDetail; displayTz: string }) {
  const name = useMemberNames();
  const q = useQuery({ queryKey: ["upcoming", s.id], queryFn: () => api.get<Transition[]>(`/schedules/${s.id}/upcoming${query({ limit: 30 })}`) });
  if (q.isLoading) return <Loading />;
  if (!q.data?.length) return <p>No hand-overs in the next six months.</p>;
  return (
    <ol className="flex flex-col gap-4">
      {q.data.map((t) => (
        <li key={`${t.layer}-${t.at}`} className="grid gap-x-6 md:grid-cols-[14rem_1fr]">
          <time dateTime={t.at} className="tabular">
            <span className="font-semibold">{formatDay(t.at, displayTz)}</span> {formatTime(t.at, displayTz)} {zoneLabel(displayTz, new Date(t.at))}
          </time>
          <span>
            <span className="font-semibold">{t.user_id ? name(t.user_id) : "Coverage ends"}</span> takes {t.layer}
            {t.previous_user_id && ` from ${name(t.previous_user_id)}`}
            {t.shift?.source.kind === "override" && " (override)"}
          </span>
        </li>
      ))}
    </ol>
  );
}

function OverrideDialog({ s, open, onClose }: { s: ScheduleDetail; open: boolean; onClose: () => void }) {
  const members = useMembers();
  const qc = useQueryClient();
  const toast = useToast();
  const now = toLocalInput(new Date(), s.timezone);
  const [form, setForm] = useState({ layer: "primary", user_id: "", start_local: now, end_local: toLocalInput(new Date(Date.now() + 86_400_000), s.timezone), reason: "" });
  const save = useMutation({
    mutationFn: () => api.post(`/schedules/${s.id}/overrides`, form),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.schedule(s.id) });
      await qc.invalidateQueries({ queryKey: keys.overview });
      toast("Override added");
      onClose();
    },
  });
  return (
    <Dialog open={open} onClose={onClose} title="Add an override">
      <p className="mb-5 text-base">Someone covers part of the rotation, for leave, holidays or an emergency swap. The rotation resumes afterwards.</p>
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <Field label="Layer" htmlFor="ov-layer">
          <Select id="ov-layer" value={form.layer} onChange={(e) => setForm({ ...form, layer: e.target.value })}>
            <option value="primary">Primary</option>
            <option value="secondary">Secondary</option>
          </Select>
        </Field>
        <Field label="Who covers" htmlFor="ov-user">
          <Select id="ov-user" required value={form.user_id} onChange={(e) => setForm({ ...form, user_id: e.target.value })}>
            <option value="">Choose a person…</option>
            {(members.data ?? []).map((m) => (
              <option key={m.user_id} value={m.user_id}>
                {m.display_name}
              </option>
            ))}
          </Select>
        </Field>
        <div className="grid gap-4 md:grid-cols-2">
          <Field label={`From (${zoneLabel(s.timezone)})`} htmlFor="ov-start">
            <Input id="ov-start" type="datetime-local" required value={form.start_local} onChange={(e) => setForm({ ...form, start_local: e.target.value })} />
          </Field>
          <Field label="Until" htmlFor="ov-end">
            <Input id="ov-end" type="datetime-local" required value={form.end_local} onChange={(e) => setForm({ ...form, end_local: e.target.value })} />
          </Field>
        </div>
        <Field label="Reason" htmlFor="ov-reason" hint="Optional. Shown in Discord.">
          <Input id="ov-reason" value={form.reason} maxLength={500} onChange={(e) => setForm({ ...form, reason: e.target.value })} placeholder="Annual leave" />
        </Field>
        {save.error && <p role="alert" className="text-danger">{errorMessage(save.error)}</p>}
        <div className="mt-2 flex justify-end gap-2">
          <Button onClick={onClose}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={save.isPending || !form.user_id}>
            Add override
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function OverridesPanel({ s, displayTz }: { s: ScheduleDetail; displayTz: string }) {
  const name = useMemberNames();
  const can = useCan();
  const qc = useQueryClient();
  const toast = useToast();
  const [cancel, setCancel] = useState<Override | null>(null);
  const cancelMut = useMutation({
    mutationFn: (o: Override) => api.delete(`/schedules/${s.id}/overrides/${o.id}`),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.schedule(s.id) });
      toast("Override canceled");
      setCancel(null);
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });
  const now = Date.now();
  const current = s.overrides.filter((o) => !o.canceled_at && new Date(o.end_at).getTime() > now);
  return (
    <Panel title="Overrides">
      {current.length === 0 ? (
        <p>No current or upcoming overrides.</p>
      ) : (
        <ul className="flex flex-col gap-4">
          {current.map((o) => (
            <li key={o.id} className="flex items-start justify-between gap-4">
              <div>
                <p className="font-semibold">
                  {name(o.user_id)} covers {o.layer}
                </p>
                <p className="text-sm tabular">
                  {formatDateTime(o.start_at, displayTz)} to {formatDateTime(o.end_at, displayTz)}
                  {o.reason && ` · ${o.reason}`}
                </p>
              </div>
              {can("manage_schedules") && (
                <Button size="sm" onClick={() => setCancel(o)}>
                  Cancel
                </Button>
              )}
            </li>
          ))}
        </ul>
      )}
      <ConfirmDialog open={!!cancel} title="Cancel this override?" confirmLabel="Cancel override" onClose={() => setCancel(null)} onConfirm={() => cancel && cancelMut.mutate(cancel)} busy={cancelMut.isPending}>
        The regular rotation applies again for this period, and Discord is told.
      </ConfirmDialog>
    </Panel>
  );
}

export function ScheduleDetailPage() {
  const { id = "" } = useParams();
  const can = useCan();
  const name = useMemberNames();
  const teams = useTeams();
  const qc = useQueryClient();
  const toast = useToast();
  const navigate = useNavigate();
  const q = useQuery({ queryKey: keys.schedule(id), queryFn: () => api.get<ScheduleDetail>(`/schedules/${id}`), refetchInterval: 60_000 });
  const [view, setView] = useState<View>("timeline");
  const [tzChoice, setTzChoice] = useState<"schedule" | "browser">("schedule");
  const [overrideOpen, setOverrideOpen] = useState(false);
  const [archiveOpen, setArchiveOpen] = useState(false);

  const range = useMemo(() => {
    const from = new Date(Date.now() - 2 * 86_400_000);
    from.setMinutes(0, 0, 0);
    return { from, to: new Date(from.getTime() + 16 * 86_400_000) };
  }, []);
  const shifts = useQuery({
    queryKey: ["assignments", id, range.from.toISOString()],
    queryFn: () => api.get<Shift[]>(`/schedules/${id}/assignments${query({ from: range.from.toISOString(), to: range.to.toISOString() })}`),
    enabled: view === "timeline",
  });

  const setStatus = useMutation({
    mutationFn: (status: "active" | "paused") => api.patch(`/schedules/${id}`, { status }),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.schedule(id) });
      toast("Schedule updated");
    },
  });
  const archive = useMutation({
    mutationFn: () => api.delete(`/schedules/${id}`),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.schedules });
      toast("Schedule archived");
      navigate("/schedules");
    },
  });

  if (q.isLoading) return <Loading />;
  if (q.error || !q.data) return <ErrorNote error={q.error ?? new Error("Schedule not found")} retry={() => q.refetch()} />;
  const s = q.data;
  const displayTz = tzChoice === "schedule" ? s.timezone : browserTimezone();
  const team = teams.data?.find((t) => t.id === s.team_id);
  const editable = can("manage_schedules") && s.status !== "archived";

  return (
    <div className="flex flex-col gap-6 md:gap-8">
      <header className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
        <div>
          <p className="text-base">
            <Link to="/schedules" className="underline-offset-4 hover:underline">
              Schedules
            </Link>
            {team && (
              <>
                {" · "}
                <Link to={`/teams/${team.id}`} className="underline-offset-4 hover:underline">
                  {team.name}
                </Link>
              </>
            )}
          </p>
          <h1 className="mt-1 text-title font-semibold">{s.name}</h1>
          <div className="mt-3 flex flex-wrap items-center gap-3">
            <Tag tone={STATE_LABEL[s.state].tone}>{STATE_LABEL[s.state].text}</Tag>
            <span className="text-base">
              {rotationText(s)} · {s.timezone}
            </span>
          </div>
          {s.description && <p className="mt-3 max-w-2xl text-lg">{s.description}</p>}
        </div>
        {editable && (
          <div className="flex flex-wrap gap-2">
            <Button variant="primary" onClick={() => setOverrideOpen(true)}>
              Add override
            </Button>
            <LinkButton to={`/schedules/${id}/edit`}>Edit</LinkButton>
            <Button onClick={() => setStatus.mutate(s.status === "paused" ? "active" : "paused")}>{s.status === "paused" ? "Resume notifications" : "Pause notifications"}</Button>
            <Button variant="ghost" onClick={() => setArchiveOpen(true)}>
              Archive
            </Button>
          </div>
        )}
      </header>

      <OnCallField s={s} displayTz={displayTz} />

      <Panel
        actions={
          <div className="flex flex-wrap items-center gap-3">
            <div role="tablist" aria-label="View" className="flex bg-surface-2 p-1">
              {(["timeline", "calendar", "list"] as View[]).map((v) => (
                <button key={v} role="tab" aria-selected={view === v} onClick={() => setView(v)} className={cx("px-3 py-1.5 text-base capitalize", view === v ? "bg-surface font-semibold" : "hover:bg-surface-3")}>
                  {v}
                </button>
              ))}
            </div>
            <label className="flex items-center gap-2 text-base">
              <span className="sr-only">Show times in</span>
              <Select className="h-9" value={tzChoice} onChange={(e) => setTzChoice(e.target.value as "schedule" | "browser")}>
                <option value="schedule">{s.timezone} (schedule)</option>
                <option value="browser">{browserTimezone()} (yours)</option>
              </Select>
            </label>
          </div>
        }
        title={view === "timeline" ? "Next two weeks" : view === "calendar" ? "Calendar" : "Upcoming hand-overs"}
      >
        {view === "timeline" &&
          (shifts.data ? <Timeline shifts={shifts.data} from={range.from} to={range.to} timezone={displayTz} name={name} now={new Date()} /> : <Loading />)}
        {view === "calendar" && <CalendarView s={s} displayTz={displayTz} />}
        {view === "list" && <UpcomingList s={s} displayTz={displayTz} />}
      </Panel>

      <div className="grid gap-6 md:gap-8 xl:grid-cols-2">
        <WhoIsOnCallAt s={s} displayTz={displayTz} />
        <OverridesPanel s={s} displayTz={displayTz} />
      </div>

      <Panel title="Configuration">
        <dl className="grid gap-x-10 gap-y-5 md:grid-cols-3">
          <div>
            <dt className="text-sm">Starts</dt>
            <dd className="font-semibold">{formatLong(s.start_at, displayTz)}</dd>
          </div>
          <div>
            <dt className="text-sm">Ends</dt>
            <dd className="font-semibold">{s.end_at ? formatLong(s.end_at, displayTz) : "Open-ended"}</dd>
          </div>
          <div>
            <dt className="text-sm">Reminders</dt>
            <dd className="font-semibold">
              {s.reminder_offsets_minutes.length ? s.reminder_offsets_minutes.map((m) => (m >= 1440 ? `${m / 1440} d` : m >= 60 ? `${m / 60} h` : `${m} min`)).join(", ") + " before" : "None"}
              {s.notify_on_boundary ? ", plus each hand-over" : ""}
            </dd>
          </div>
          <div className="md:col-span-3">
            <dt className="text-sm">Rotation history</dt>
            <dd>
              <ol className="mt-2 flex max-w-3xl flex-col gap-2">
                {[...s.versions].reverse().map((v) => (
                  <li key={v.id} className="text-base">
                    <span className="font-semibold">From {formatDateTime(v.effective_from, displayTz)}:</span> every {v.interval} {v.unit}; primary {v.primary.map(name).join(" → ") || "nobody"}
                    {v.secondary.length > 0 && `; secondary ${v.secondary.map(name).join(" → ")}`}
                  </li>
                ))}
              </ol>
            </dd>
          </div>
        </dl>
      </Panel>

      <OverrideDialog key={String(overrideOpen)} s={s} open={overrideOpen} onClose={() => setOverrideOpen(false)} />
      <ConfirmDialog open={archiveOpen} title={`Archive ${s.name}?`} confirmLabel="Archive schedule" onClose={() => setArchiveOpen(false)} onConfirm={() => archive.mutate()} busy={archive.isPending}>
        Nobody will be on call from this schedule and its pending notifications are withdrawn. Its history and audit trail are kept.
      </ConfirmDialog>
    </div>
  );
}
