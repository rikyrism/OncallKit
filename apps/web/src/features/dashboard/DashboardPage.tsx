import { Link } from "react-router";
import { ArrowRight } from "lucide-react";
import { useCan, useMe, useMemberNames, useNotificationHealth, useOverview, useSwaps, useTeams } from "@/lib/queries";
import { Empty, ErrorNote, LinkButton, Loading, Panel, cx } from "@/components/ui";
import { formatDateTime, formatDay, formatTime, relative, zoneLabel } from "@/lib/time";
import type { Overview } from "@/lib/types";

/** The duty board: whoever holds the pager is the headline, on a field of signal yellow. */
function DutyBoard({ schedules }: { schedules: Overview["schedules"] }) {
  const name = useMemberNames();
  const live = schedules.filter((s) => s.on_call.primary);
  const idle = schedules.filter((s) => !s.on_call.primary);
  const single = live.length === 1;
  return (
    <section aria-label="On call now" className="rise bg-signal text-signal-ink">
      {live.length === 0 && <p className="px-5 py-8 text-3xl font-semibold tracking-[-0.02em] md:px-8">Nobody is on call right now.</p>}
      <ul className="flex flex-col">
        {live.map((s, i) => {
          const p = s.on_call.primary!;
          const sec = s.on_call.secondary;
          return (
            <li key={s.id} className={cx("px-5 py-6 md:px-8 md:py-8", i > 0 && "bg-black/[0.06]")}>
              <Link to={`/schedules/${s.id}`} className="group grid gap-x-10 gap-y-4 md:grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)] md:items-end">
                <div className="min-w-0">
                  <p className={cx("font-semibold tracking-[-0.035em] group-hover:underline group-hover:decoration-2 group-hover:underline-offset-8", single ? "text-display" : "text-[clamp(3rem,5.5vw,5.25rem)] leading-[0.95]")}>
                    {name(p.user_id)}
                  </p>
                  <p className="mt-2 text-lg">
                    On call now, primary on {s.name}
                    {p.source.kind === "override" && ", covering by override"}
                  </p>
                </div>
                <dl className="grid grid-cols-2 gap-x-6 gap-y-1 text-base">
                  <dt>Secondary</dt>
                  <dt>Hands over</dt>
                  <dd className="text-lg font-semibold">{sec ? name(sec.user_id) : "None"}</dd>
                  <dd className="text-lg font-semibold tabular">
                    {s.on_call.next_rotation_at ? (
                      <time dateTime={s.on_call.next_rotation_at}>
                        {formatDay(s.on_call.next_rotation_at, s.timezone)}, {formatTime(s.on_call.next_rotation_at, s.timezone)}
                      </time>
                    ) : (
                      "Schedule ends"
                    )}
                  </dd>
                  <dd className="col-span-2 mt-2">
                    Since {formatDateTime(p.start, s.timezone)} {zoneLabel(s.timezone)}
                  </dd>
                </dl>
              </Link>
            </li>
          );
        })}
      </ul>
      {idle.length > 0 && (
        <p className="px-5 pb-5 text-base md:px-8">
          Not covering right now: {idle.map((s) => `${s.name} (${s.state.replace("_", " ")})`).join(", ")}
        </p>
      )}
    </section>
  );
}

function Upcoming({ items }: { items: Overview["upcoming"] }) {
  const name = useMemberNames();
  return (
    <Panel title="Next hand-overs" className="rise rise-2">
      {items.length === 0 ? (
        <p>No hand-overs in the next 30 days.</p>
      ) : (
        <ol className="flex flex-col gap-5">
          {items.slice(0, 8).map((u) => (
            <li key={`${u.schedule_id}-${u.layer}-${u.at}`} className="grid grid-cols-[7.5rem_1fr] gap-4">
              <time dateTime={u.at} className="tabular">
                <span className="block font-semibold">{formatDay(u.at, u.timezone)}</span>
                <span className="block whitespace-nowrap text-sm">
                  {formatTime(u.at, u.timezone)} {zoneLabel(u.timezone)}
                </span>
              </time>
              <div className="min-w-0">
                <p className="text-lg font-semibold leading-tight">{u.user_id ? name(u.user_id) : "Coverage ends"}</p>
                <p className="text-sm text-ink-soft">
                  {u.layer === "primary" ? "Primary" : "Secondary"} on {u.schedule_name}
                  {u.previous_user_id ? `, after ${name(u.previous_user_id)}` : ""} · {relative(u.at)}
                </p>
              </div>
            </li>
          ))}
        </ol>
      )}
    </Panel>
  );
}

function Health() {
  const { data } = useNotificationHealth();
  const can = useCan();
  if (!data) return null;
  const stats = [
    { label: "Delivered, 24 h", value: data.succeeded_24h },
    { label: "Failed, 24 h", value: data.failed_24h, bad: data.failed_24h > 0 },
    { label: "Waiting to retry", value: data.retrying },
  ];
  return (
    <Panel
      title="Discord delivery"
      className="rise rise-3"
      actions={
        can("view_integrations") && (
          <Link to="/notifications" className="inline-flex items-center gap-1 text-base underline-offset-4 hover:underline">
            History <ArrowRight className="size-4" aria-hidden />
          </Link>
        )
      }
    >
      {data.integrations === 0 ? (
        <p>
          No Discord channel is connected, so hand-overs are not announced.
          {can("manage_integrations") && (
            <>
              {" "}
              <Link to="/integrations" className="font-medium underline underline-offset-4">
                Connect one
              </Link>
              .
            </>
          )}
        </p>
      ) : (
        <>
          <dl className="grid grid-cols-3 gap-4">
            {stats.map((s) => (
              <div key={s.label}>
                <dd className={cx("text-4xl font-semibold tracking-[-0.03em] tabular", s.bad && "text-danger")}>{s.value}</dd>
                <dt className="text-sm">{s.label}</dt>
              </div>
            ))}
          </dl>
          <p className="mt-5 text-base">
            {data.integrations_enabled} of {data.integrations} channels enabled.
            {data.last_success_at && <> Last delivered {relative(data.last_success_at)}.</>}
          </p>
        </>
      )}
    </Panel>
  );
}

function Teams() {
  const { data: teams } = useTeams();
  const { data: overview } = useOverview();
  const name = useMemberNames();
  if (!teams?.length) return null;
  return (
    <Panel title="Teams" className="rise rise-4">
      <div className="overflow-x-auto">
        <table className="data-table">
          <thead>
            <tr>
              <th>Team</th>
              <th>People</th>
              <th>Active schedules</th>
              <th>On call now</th>
            </tr>
          </thead>
          <tbody>
            {teams.map((t) => {
              const onCall = (overview?.schedules ?? []).filter((s) => s.team_id === t.id && s.on_call.primary).map((s) => name(s.on_call.primary!.user_id));
              return (
                <tr key={t.id}>
                  <td>
                    <Link to={`/teams/${t.id}`} className="font-semibold underline-offset-4 hover:underline">
                      {t.name}
                    </Link>
                  </td>
                  <td className="tabular">{t.member_ids.length}</td>
                  <td className="tabular">{t.active_schedule_count}</td>
                  <td>{onCall.length ? Array.from(new Set(onCall)).join(", ") : "Nobody"}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </Panel>
  );
}

/** Swap requests waiting for the signed-in person's answer. */
function SwapsWaiting() {
  const { data: me } = useMe();
  const { data } = useSwaps();
  const name = useMemberNames();
  const waiting = (data ?? []).filter((s) => s.status === "pending" && s.target_user_id === me?.user.user_id);
  const first = waiting[0];
  if (!first) return null;
  return (
    <Link to="/swaps" className="group flex flex-wrap items-center justify-between gap-3 bg-surface px-5 py-4 md:px-7">
      <span className="text-lg">
        <strong>{name(first.requester_id)}</strong> asked you to cover a shift on {first.schedule_name}
        {waiting.length > 1 && ` (and ${waiting.length - 1} more request${waiting.length > 2 ? "s" : ""})`}.
      </span>
      <span className="inline-flex items-center gap-2 font-semibold group-hover:underline group-hover:underline-offset-4">
        Answer <ArrowRight className="size-4" />
      </span>
    </Link>
  );
}

export function DashboardPage() {
  const { data: me } = useMe();
  const { data, isLoading, error, refetch } = useOverview();
  const can = useCan();
  const org = me?.memberships.find((m) => m.organization_id === me.active_organization_id);

  return (
    <div className="flex flex-col gap-6 md:gap-8">
      <header className="flex flex-col gap-4 md:flex-row md:items-end md:justify-between">
        <h1 className="text-title font-semibold">{org?.name ?? "Overview"}</h1>
        <div className="flex flex-wrap gap-2">
          {can("manage_schedules") && (
            <LinkButton to="/schedules/new" variant="primary">Create schedule</LinkButton>
          )}
          {can("manage_members") && (
            <LinkButton to="/users" className="max-md:hidden">Invite member</LinkButton>
          )}
          {can("manage_teams") && (
            <LinkButton to="/teams" className="max-md:hidden">Add team</LinkButton>
          )}
          {can("manage_integrations") && (
            <LinkButton to="/integrations" className="max-md:hidden">Connect Discord</LinkButton>
          )}
        </div>
      </header>

      <SwapsWaiting />
      {isLoading && <Loading />}
      {error && <ErrorNote error={error} retry={() => refetch()} />}
      {data && data.schedules.length === 0 && (
        <Empty
          title="No schedules yet"
          action={
            can("manage_schedules") ? (
              <LinkButton to="/schedules/new" variant="primary">Create the first schedule</LinkButton>
            ) : undefined
          }
        >
          A schedule decides who is on call and when it hands over. Add the people in the order they should rotate, choose how often, and OncallKit works
          out every shift from there, including who is on call right now. The{" "}
          <Link to="/guide" className="font-semibold underline underline-offset-4">
            setup guide
          </Link>{" "}
          walks you from here to messages in Discord.
        </Empty>
      )}
      {data && data.schedules.length > 0 && (
        <>
          <DutyBoard schedules={data.schedules} />
          <div className="grid gap-6 md:gap-8 xl:grid-cols-[1.4fr_1fr]">
            <Upcoming items={data.upcoming} />
            <Health />
          </div>
          <Teams />
        </>
      )}
    </div>
  );
}
