import { useEffect, useRef, useState, type ReactNode } from "react";
import { Link } from "react-router";
import { ArrowRight, Check, GripVertical } from "lucide-react";
import { useCan, useIntegrations, useMe, useMemberNames, useMembers, useOverview, useSchedules, useTeams } from "@/lib/queries";
import type { Member, Overview } from "@/lib/types";
import { cx } from "@/components/ui";
import { zoneLabel } from "@/lib/time";

/**
 * The setup guide: from nothing to on-call messages in Discord. The message the channel will
 * receive is the page's centrepiece, filled in from the organization's own schedules; each step
 * marks itself done from the organization's real state and shows the screen it happens on.
 */

type StepState = "done" | "open" | "untracked";

// ---------- motion: parts of each step rise in as the step scrolls into view ----------

function useReveal<T extends Element>() {
  const ref = useRef<T>(null);
  const [shown, setShown] = useState(() => typeof IntersectionObserver === "undefined");
  useEffect(() => {
    const el = ref.current;
    if (!el || shown) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setShown(true);
          io.disconnect();
        }
      },
      { rootMargin: "0px 0px -10% 0px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [shown]);
  /** Class for the nth part (0, 1, 2…) of the revealed block. */
  const part = (n: number) => (shown ? cx("rise", n === 1 && "rise-2", n === 2 && "rise-3", n >= 3 && "rise-4") : "reveal-wait");
  return { ref, part };
}

// ---------- small pieces ----------

/** A path through menus, shown as the words on the buttons to press. */
function Path({ parts }: { parts: string[] }) {
  return (
    <p className="flex flex-wrap items-center gap-x-1.5 gap-y-2 text-base">
      {parts.map((p, i) => (
        <span key={p} className="inline-flex items-center gap-1.5">
          {i > 0 && <ArrowRight aria-hidden className="size-4 text-ink-soft" />}
          <span className="bg-surface-2 px-2 py-0.5 font-medium">{p}</span>
        </span>
      ))}
    </p>
  );
}

function StateTag({ state }: { state: StepState }) {
  if (state === "untracked") return null;
  return state === "done" ? (
    <span className="inline-flex items-center gap-1 bg-signal px-2 py-0.5 text-base font-semibold text-signal-ink">
      <Check aria-hidden className="size-4" strokeWidth={2.5} /> Done
    </span>
  ) : (
    <span className="bg-surface-2 px-2 py-0.5 text-base font-medium">To do</span>
  );
}

function discordTime(iso: string, tz: string) {
  const text = new Intl.DateTimeFormat("en-GB", { timeZone: tz, day: "numeric", month: "long", year: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(
    new Date(iso),
  );
  return `${text.replace(" at", ",")} ${zoneLabel(tz, new Date(iso))}`;
}

function shortTime(iso: string, tz: string) {
  return new Intl.DateTimeFormat("en-GB", { timeZone: tz, hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(new Date(iso));
}

// ---------- drawings of the real screens (Discord's and OncallKit's own interface) ----------

const discordMention = "rounded-[3px] bg-[#5865f24d] px-0.5 text-[#c9cdfb]";

function BotAvatar() {
  return (
    <span aria-hidden className="mt-0.5 inline-flex size-10 shrink-0 items-center justify-center rounded-full bg-signal">
      <svg viewBox="0 0 32 32" className="size-6">
        <path d="M9 23V9h4v10h10v4z" fill="#141413" />
      </svg>
    </span>
  );
}

interface EmbedField {
  name: string;
  value: ReactNode;
  inline: boolean;
}

/** A message from the OncallKit webhook as Discord draws it, in Discord's own colours. */
function DiscordMessage({ channel, time, title, fields, tag, label }: { channel: string; time: string; title: string; fields: EmbedField[]; tag?: string | null; label: string }) {
  return (
    <figure data-ui aria-label={label} className="flex flex-col bg-[#313338] text-[15px] leading-[1.375] text-[#dbdee1]">
      <figcaption className="flex items-center gap-2 bg-[#2b2d31] px-4 py-3 font-semibold text-[#f2f3f5]">
        <span aria-hidden className="text-xl leading-none text-[#80848e]">
          #
        </span>
        {channel}
      </figcaption>
      <div className="flex gap-4 px-4 py-4">
        <BotAvatar />
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-baseline gap-x-2">
            <span className="font-semibold text-[#f2f3f5]">OncallKit</span>
            <span className="rounded-[3px] bg-[#5865f2] px-1 text-[10px] font-semibold leading-4 text-white">APP</span>
            <span className="text-xs text-[#949ba4]">Today at {time}</span>
          </p>
          {tag && (
            <p className="mt-0.5">
              <span className={discordMention}>@{tag}</span>
            </p>
          )}
          <div className="mt-1.5 flex max-w-[520px] overflow-hidden rounded-[4px] bg-[#2b2d31]">
            <span aria-hidden className="w-1 shrink-0 bg-signal" />
            <div className="min-w-0 px-4 pb-4 pt-3">
              <p className="font-semibold text-[#00a8fc]">{title}</p>
              <dl className="mt-2 grid grid-cols-2 gap-x-6 gap-y-2 sm:grid-cols-3">
                {fields.map((f) => (
                  <div key={f.name} className={f.inline ? "" : "col-span-2 sm:col-span-3"}>
                    <dt className="text-[13px] font-semibold text-[#f2f3f5]">{f.name}</dt>
                    <dd className="text-[14px]">{f.value}</dd>
                  </div>
                ))}
              </dl>
              <p className="mt-3 text-xs text-[#949ba4]">OncallKit</p>
            </div>
          </div>
        </div>
      </div>
    </figure>
  );
}

/** Discord: Server Settings, Integrations, Webhooks, with a new webhook opened. */
function DiscordWebhooks({ channel }: { channel: string }) {
  return (
    <figure data-ui aria-label="Discord's webhook settings with Copy Webhook URL highlighted" className="flex flex-col gap-4 bg-[#313338] p-5 text-[14px] text-[#dbdee1]">
      <p className="text-[20px] font-semibold text-[#f2f3f5]">Webhooks</p>
      <p className="text-[#b5bac1]">Webhooks are a simple way to post messages from other apps and websites into Discord.</p>
      <span className="w-fit rounded-[3px] bg-[#5865f2] px-4 py-2 font-medium text-white">New Webhook</span>
      <div className="flex flex-col gap-4 rounded-[8px] bg-[#2b2d31] p-4">
        <div className="flex items-center gap-3">
          <BotAvatar />
          <div>
            <p className="font-semibold text-[#f2f3f5]">OncallKit</p>
            <p className="text-xs text-[#949ba4]">Created today</p>
          </div>
        </div>
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="flex flex-col gap-1">
            <span className="text-[12px] font-bold text-[#b5bac1]">NAME</span>
            <span className="rounded-[3px] bg-[#1e1f22] px-3 py-2">OncallKit</span>
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-[12px] font-bold text-[#b5bac1]">CHANNEL</span>
            <span className="rounded-[3px] bg-[#1e1f22] px-3 py-2"># {channel}</span>
          </div>
        </div>
        <div className="flex flex-wrap gap-3">
          <span className="rounded-[3px] bg-[#4e5058] px-4 py-2 font-medium text-white ring-2 ring-signal ring-offset-2 ring-offset-[#2b2d31]">Copy Webhook URL</span>
          <span className="rounded-[3px] px-4 py-2 font-medium text-[#ff8082]">Delete Webhook</span>
        </div>
      </div>
    </figure>
  );
}

function FakeCheck({ on, label }: { on: boolean; label: string }) {
  return (
    <span className="flex items-center gap-2.5">
      <span aria-hidden className={cx("inline-flex size-4 items-center justify-center", on ? "bg-ink text-ground" : "border-2 border-ink-soft")}>
        {on && <Check className="size-3" strokeWidth={3} />}
      </span>
      {label}
    </span>
  );
}

/** OncallKit: the Connect Discord form, filled in. */
function ConnectForm({ channel }: { channel: string }) {
  return (
    <figure data-ui aria-label="OncallKit's Connect Discord form, filled in" className="flex flex-col gap-4 bg-surface p-5 text-base md:p-6">
      <p className="text-2xl font-semibold tracking-[-0.02em]">Connect Discord</p>
      <div className="flex flex-col gap-1.5">
        <span className="font-medium">Name</span>
        <span className="border border-surface-3 bg-ground px-3 py-2">#{channel}</span>
      </div>
      <div className="flex flex-col gap-1.5">
        <span className="font-medium">Webhook URL</span>
        <span className="truncate border border-surface-3 bg-ground px-3 py-2">https://discord.com/api/webhooks/1234567890/••••••••</span>
      </div>
      <div className="flex flex-col gap-2">
        <span className="font-medium">Events</span>
        <FakeCheck on label="Rotation hand-over" />
        <FakeCheck on label="Reminder before a shift" />
        <FakeCheck on={false} label="Schedule updated" />
      </div>
      <FakeCheck on label="Mention people" />
      <div className="flex gap-2">
        <span className="bg-signal px-4 py-2 font-semibold text-signal-ink">Save</span>
        <span className="bg-surface-2 px-4 py-2 font-semibold ring-2 ring-signal ring-offset-2 ring-offset-surface">Send test</span>
      </div>
    </figure>
  );
}

/** OncallKit: the invitation dialog with its one-time link. */
function InviteForm() {
  return (
    <figure data-ui aria-label="OncallKit's invitation dialog showing a one-time link" className="flex flex-col gap-4 bg-surface p-5 text-base md:p-6">
      <p className="text-2xl font-semibold tracking-[-0.02em]">Invite someone</p>
      <p>
        Send this link to <strong>sari@acme.test</strong>. It works only for that Google account and expires in 7 days. It is shown once.
      </p>
      <span className="truncate border border-surface-3 bg-ground px-3 py-2">https://oncall.acme.test/invite/k3J9…</span>
      <div className="flex justify-end gap-2">
        <span className="bg-signal px-4 py-2 font-semibold text-signal-ink">Copy link</span>
        <span className="bg-surface-2 px-4 py-2 font-semibold">Done</span>
      </div>
    </figure>
  );
}

/** OncallKit: the parts of the schedule form that matter, with the organization's own people. */
function ScheduleForm({ people }: { people: string[] }) {
  return (
    <figure data-ui aria-label="OncallKit's schedule form: start, rotation length, order and reminders" className="flex flex-col gap-4 bg-surface p-5 text-base md:p-6">
      <div className="grid grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)] gap-3">
        <div className="flex flex-col gap-1.5">
          <span className="font-medium">Starts</span>
          <span className="border border-surface-3 bg-ground px-3 py-2 tabular">Mon 2 Nov, 09:00</span>
        </div>
        <div className="flex flex-col gap-1.5">
          <span className="font-medium">Every</span>
          <span className="border border-surface-3 bg-ground px-3 py-2 tabular">7 days</span>
        </div>
      </div>
      <div className="flex flex-col gap-1.5">
        <span className="font-medium">Primary rotation</span>
        <ol className="flex flex-col gap-1">
          {people.map((p) => (
            <li key={p} className="flex items-center gap-2 bg-surface-2 px-3 py-2">
              <GripVertical aria-hidden className="size-4 text-ink-soft" />
              {p}
            </li>
          ))}
        </ol>
      </div>
      <div className="flex flex-col gap-2">
        <span className="font-medium">Remind the next person</span>
        <FakeCheck on label="24 hours before" />
        <FakeCheck on label="1 hour before" />
        <FakeCheck on={false} label="15 minutes before" />
      </div>
    </figure>
  );
}

/** Discord: the menu on a person's name, with Copy User ID. */
function DiscordUserMenu({ person, id }: { person: string; id: string }) {
  return (
    <figure data-ui aria-label="Discord's menu on a person, with Copy User ID highlighted" className="flex flex-col gap-4 bg-[#313338] p-5 text-[14px] text-[#dbdee1] sm:flex-row sm:items-start">
      <p className="flex items-center gap-3">
        <span aria-hidden className="inline-flex size-10 items-center justify-center rounded-full bg-[#5865f2] text-base font-semibold text-white">
          {person.charAt(0)}
        </span>
        <span className="font-semibold text-[#f2f3f5]">{person}</span>
      </p>
      <ul className="flex w-full max-w-[240px] flex-col rounded-[4px] bg-[#111214] p-1.5 shadow-2xl">
        {["Profile", "Mention", "Message", "Add Note"].map((item) => (
          <li key={item} className="px-2 py-1.5 text-[#b5bac1]">
            {item}
          </li>
        ))}
        <li className="mt-1 flex items-center justify-between gap-3 rounded-[2px] bg-[#4752c4] px-2 py-1.5 text-white">
          Copy User ID <span className="truncate text-[11px] opacity-80">{id}</span>
        </li>
      </ul>
    </figure>
  );
}

// ---------- the live message ----------

interface Live {
  schedule: string;
  team: string | null;
  tz: string;
  primary: string;
  primaryTag: string | null;
  secondary: string | null;
  at: string;
  next: string | null;
  example: boolean;
}

function liveFrom(overview: Overview | undefined, members: Member[] | undefined, name: (id: string) => string, teamName: (id: string | null) => string | null, mentionOn: boolean): Live {
  const live = overview?.schedules.find((s) => s.on_call.primary);
  if (live?.on_call.primary) {
    const p = live.on_call.primary;
    const discord = members?.find((m) => m.user_id === p.user_id)?.discord_user_id ?? null;
    return {
      schedule: live.name,
      team: teamName(live.team_id),
      tz: live.timezone,
      primary: name(p.user_id),
      primaryTag: mentionOn && discord ? name(p.user_id) : null,
      secondary: live.on_call.secondary ? name(live.on_call.secondary.user_id) : null,
      at: p.start,
      next: live.on_call.next_rotation_at,
      example: false,
    };
  }
  const monday = new Date();
  monday.setUTCDate(monday.getUTCDate() + ((8 - monday.getUTCDay()) % 7 || 7));
  monday.setUTCHours(2, 0, 0, 0);
  return {
    schedule: "Platform on-call",
    team: "Platform",
    tz: "Asia/Jakarta",
    primary: "Andi",
    primaryTag: null,
    secondary: "Sari",
    at: monday.toISOString(),
    next: new Date(monday.getTime() + 7 * 86_400_000).toISOString(),
    example: true,
  };
}

function handoverFields(l: Live): EmbedField[] {
  return [
    {
      name: "Primary",
      value: l.primaryTag ? (
        <>
          {l.primary} (<span className={discordMention}>@{l.primaryTag}</span>)
        </>
      ) : (
        l.primary
      ),
      inline: true,
    },
    ...(l.secondary ? [{ name: "Secondary", value: l.secondary, inline: true }] : []),
    { name: "Effective from", value: discordTime(l.at, l.tz), inline: false },
    ...(l.next ? [{ name: "Next rotation", value: discordTime(l.next, l.tz), inline: false }] : []),
    ...(l.team ? [{ name: "Team", value: l.team, inline: true }] : []),
    { name: "Schedule", value: l.schedule, inline: true },
  ];
}

// ---------- a step ----------

function Step({ id, title, state, picture, flip, children, action }: { id: string; title: string; state: StepState; picture: ReactNode; flip?: boolean; children: ReactNode; action?: { to: string; label: string } }) {
  const { ref, part } = useReveal<HTMLLIElement>();
  return (
    <li ref={ref} id={id} className="grid scroll-mt-6 items-start gap-8 lg:grid-cols-2 lg:gap-14">
      <div className={cx("flex min-w-0 flex-col gap-4 text-lg", flip && "lg:order-2")}>
        <div className={cx("flex flex-wrap items-center gap-x-4 gap-y-2", part(0))}>
          <h2 className="text-title font-semibold">{title}</h2>
          <StateTag state={state} />
        </div>
        <div className={cx("flex flex-col gap-4", part(1))}>
          {children}
          {action && (
            <Link to={action.to} className="inline-flex w-fit items-center gap-2 bg-surface-2 px-4 py-2.5 text-base font-semibold transition-[background-color,transform] hover:-translate-y-0.5 hover:bg-surface-3 active:translate-y-0">
              {action.label} <ArrowRight aria-hidden className="size-4" />
            </Link>
          )}
        </div>
      </div>
      <div className={cx("min-w-0", flip && "lg:order-1", part(2))}>{picture}</div>
    </li>
  );
}

// ---------- the page ----------

export function GuidePage() {
  const { data: me } = useMe();
  const can = useCan();
  const admin = can("view_integrations");
  const integrations = useIntegrations(admin);
  const members = useMembers();
  const schedules = useSchedules();
  const overview = useOverview();
  const teams = useTeams();
  const name = useMemberNames();
  const close = useReveal<HTMLElement>();

  const ints = integrations.data ?? [];
  const mentionOn = ints.some((i) => i.enabled && i.mention_users);
  const known = (ready: boolean, ok: boolean): StepState => (!ready ? "untracked" : ok ? "done" : "open");
  const teamName = (tid: string | null) => (tid ? ((teams.data ?? []).find((t) => t.id === tid)?.name ?? null) : null);
  const live = liveFrom(overview.data, members.data, name, teamName, mentionOn);
  const channel = ints.find((i) => i.enabled)?.name.replace(/^#/, "") || "on-call";
  const roster = (members.data ?? []).map((m) => m.display_name).slice(0, 3);
  const people = roster.length >= 2 ? roster : ["Andi", "Budi", "Sari"];
  const myId = me?.user.user_id;
  const ownId = members.data?.find((m) => m.user_id === myId)?.discord_user_id;
  const covering = people[1] ?? "Budi";
  const coverStart = new Date(Math.ceil(Date.now() / 900_000) * 900_000).toISOString();
  const coverEnd = new Date(Date.parse(coverStart) + 3_600_000).toISOString();

  const states = {
    webhook: known(admin && !!integrations.data, ints.length > 0),
    connect: known(admin && !!integrations.data, ints.some((i) => i.last_delivery_status === "succeeded")),
    people: known(!!members.data, (members.data?.length ?? 0) >= 2),
    schedule: known(!!schedules.data, (schedules.data?.length ?? 0) > 0),
    mentions: known(admin && !!members.data && !!integrations.data, mentionOn && (members.data ?? []).some((m) => m.discord_user_id)),
  };
  const titles: Record<keyof typeof states, string> = {
    webhook: "Create a webhook in Discord",
    connect: "Connect it to OncallKit",
    people: "Bring your team in",
    schedule: "Create the rotation",
    mentions: "Tag people in Discord",
  };
  const trackedKeys = (Object.keys(states) as (keyof typeof states)[]).filter((k) => states[k] !== "untracked");
  const done = trackedKeys.filter((k) => states[k] === "done").length;
  const nextKey = trackedKeys.find((k) => states[k] === "open");

  return (
    <div className="flex flex-col gap-20 md:gap-32">
      <section className="flex flex-col gap-10">
        <h1 className="rise text-display font-semibold text-balance">From zero to a Discord ping</h1>
        <div className="grid items-start gap-10 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)] xl:gap-14">
          <div className="rise rise-2 flex min-w-0 flex-col gap-6">
            <p className="max-w-xl text-xl">
              Six steps, about ten minutes. Then your team's channel hears who is on call the moment the pager moves, with a reminder before every shift.
            </p>
            {trackedKeys.length > 0 && (
              <div className="flex flex-col gap-4">
                <p className="text-lg">
                  <strong className="tabular">
                    {done} of {trackedKeys.length}
                  </strong>{" "}
                  {done === trackedKeys.length ? "done. Your organization is fully set up." : "done in your organization so far."}
                </p>
                <div aria-hidden className="flex h-2 w-full max-w-md gap-1">
                  {trackedKeys.map((k) => (
                    <span key={k} className={cx("flex-1 transition-colors", states[k] === "done" ? "bg-signal" : "bg-surface-3")} />
                  ))}
                </div>
                {nextKey && (
                  <a
                    href={`#${nextKey}`}
                    className="inline-flex w-fit items-center gap-2 bg-signal px-5 py-3 font-semibold text-signal-ink transition-transform hover:-translate-y-0.5 active:translate-y-0"
                  >
                    Next: {titles[nextKey].charAt(0).toLowerCase() + titles[nextKey].slice(1)} <ArrowRight aria-hidden className="size-4" />
                  </a>
                )}
              </div>
            )}
          </div>
          <div className="rise rise-3 flex min-w-0 flex-col gap-3">
            <DiscordMessage
              label="The hand-over message in Discord"
              channel={channel}
              time={shortTime(live.at, live.tz)}
              title={`${live.schedule}: On-call handover`}
              fields={handoverFields(live)}
              tag={live.primaryTag}
            />
            <p className="text-base text-ink-soft">
              {live.example
                ? "An example of the hand-over message. Once a schedule is running, this shows your own."
                : `The hand-over message as your channel would see it right now, from ${live.schedule}.${live.primaryTag ? "" : " Tag people (below) to turn the name into a mention."}`}
            </p>
          </div>
        </div>
      </section>

      <ol className="flex list-none flex-col gap-20 md:gap-28">
        <Step id="webhook" title={titles.webhook} state={states.webhook} picture={<DiscordWebhooks channel={channel} />}>
          <p>A webhook is a private address that lets OncallKit post into one channel. You need the Manage Webhooks permission on the Discord server.</p>
          <Path parts={["Server Settings", "Integrations", "Webhooks", "New Webhook"]} />
          <p>
            Pick the channel the team watches, name the webhook OncallKit if you like, and press <strong>Copy Webhook URL</strong>. Treat that link like a password: anyone who has it
            can post in the channel.
          </p>
        </Step>

        <Step id="connect" title={titles.connect} state={states.connect} flip picture={<ConnectForm channel={channel} />} action={admin ? { to: "/integrations", label: "Open Integrations" } : undefined}>
          <Path parts={["Integrations", "Connect Discord"]} />
          <p>
            Paste the webhook URL and choose what this channel should hear. <strong>Rotation hand-over</strong> and <strong>Reminder before a shift</strong> are the pair most teams want.
            Save, then press <strong>Send test</strong>. A test message in Discord means it works; if it fails, OncallKit shows exactly what Discord answered.
          </p>
          {!admin && <p className="text-ink-soft">Owners and admins connect channels. Ask one of them to do this step.</p>}
        </Step>

        <Step id="people" title={titles.people} state={states.people} picture={<InviteForm />} action={can("manage_members") ? { to: "/users", label: "Open Users" } : undefined}>
          <Path parts={["Users", "Invite member"]} />
          <p>
            Send each person their invitation link. They sign in with the Google account of that address and land straight in this organization. Teams are optional: they let a
            channel hear only about its own team's schedules.
          </p>
        </Step>

        <Step
          id="schedule"
          title={titles.schedule}
          state={states.schedule}
          flip
          picture={<ScheduleForm people={people} />}
          action={can("manage_schedules") ? { to: "/schedules/new", label: "Create schedule" } : undefined}
        >
          <Path parts={["Schedules", "Create schedule"]} />
          <dl className="grid gap-x-6 gap-y-3 sm:grid-cols-[minmax(0,9rem)_minmax(0,1fr)]">
            <dt className="font-semibold">Starts</dt>
            <dd>The first hand-over. Every later one happens at the same time of day, in the schedule's timezone.</dd>
            <dt className="font-semibold">Every</dt>
            <dd>How long one person holds the pager: 7 days for a weekly rotation, 12 hours for day and night shifts.</dd>
            <dt className="font-semibold">Primary rotation</dt>
            <dd>The people in the order they take turns. A secondary rotation is an optional backup.</dd>
            <dt className="font-semibold">Remind</dt>
            <dd>1 week, 24 hours, 1 hour or 15 minutes before a shift starts.</dd>
          </dl>
        </Step>

        <Step
          id="mentions"
          title={titles.mentions}
          state={states.mentions}
          picture={<DiscordUserMenu person={covering} id="123456789012345678" />}
          action={can("manage_members") ? { to: "/users", label: "Set Discord ids" } : undefined}
        >
          <p>Optional, and worth it: names become real @mentions, so the person going on call gets a notification on their phone.</p>
          <p>In Discord, turn on Developer Mode, then right-click the person and choose Copy User ID.</p>
          <Path parts={["User Settings", "Advanced", "Developer Mode"]} />
          <p>Paste it next to their name in OncallKit, then turn on Mention people for the channel.</p>
          <Path parts={["Users", "Discord", "Set"]} />
          <Path parts={["Integrations", "Edit", "Mention people"]} />
          {!ownId && (
            <p>
              Your own id is not set yet.{" "}
              <Link to="/profile" className="font-semibold underline underline-offset-4">
                Add it on your profile
              </Link>
              .
            </p>
          )}
        </Step>

        <Step
          id="try"
          title="Watch it arrive"
          state="untracked"
          flip
          picture={
            <DiscordMessage
              label="The override message in Discord"
              channel={channel}
              time={shortTime(coverStart, live.tz)}
              title={`${live.schedule}: override added`}
              tag={live.primaryTag ? covering : null}
              fields={[
                { name: "Primary cover", value: covering, inline: true },
                { name: "Instead of", value: live.primary === covering ? (people[0] ?? "Andi") : live.primary, inline: true },
                { name: "From", value: discordTime(coverStart, live.tz), inline: false },
                { name: "Until", value: discordTime(coverEnd, live.tz), inline: false },
                { name: "Reason", value: "Trying OncallKit", inline: false },
              ]}
            />
          }
          action={can("view_integrations") ? { to: "/notifications", label: "Open Notification history" } : undefined}
        >
          <Path parts={["Schedules", "your schedule", "Add override"]} />
          <p>
            Choose who covers, from now until an hour later, and save. Within a minute the channel shows the override and the new person on call. From then on, hand-overs and reminders
            arrive by themselves at the times the schedule sets. Cancel the override to hand the pager back.
          </p>
        </Step>
      </ol>

      <section ref={close.ref} className="grid gap-8 bg-signal p-6 text-signal-ink md:grid-cols-[minmax(0,1fr)_minmax(0,1.6fr)] md:p-10">
        <h2 className={cx("text-title font-semibold", close.part(0))}>Nothing arrived?</h2>
        <ul className={cx("flex list-none flex-col gap-4 text-lg", close.part(1))}>
          <li>
            <strong>Check Notification history.</strong> Every message is listed with Discord's answer. A 404 means the webhook was deleted in Discord: create a new one and paste it under
            Edit. A 429 means Discord asked to slow down; OncallKit retries by itself.
          </li>
          <li>
            <strong>Check the schedule is not paused.</strong> A paused schedule still shows who is on call but sends nothing until you resume it.
          </li>
          <li>
            <strong>Check the channel listens for it.</strong> Under Integrations, Edit, the event must be ticked, and a team channel only hears about that team's schedules.
          </li>
          <li>
            <strong>Mentions show as plain names?</strong> The person needs a Discord id and the channel needs Mention people turned on.
          </li>
        </ul>
      </section>
    </div>
  );
}
