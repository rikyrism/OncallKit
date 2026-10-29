import { useMemo, useState, type ReactNode } from "react";
import { Link } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage, query } from "@/lib/api";
import { keys, useCan, useMe, useMemberNames, useMembers, useSchedules, useSwaps } from "@/lib/queries";
import type { Shift, SwapRequest, SwapStatus } from "@/lib/types";
import {
  Button,
  Dialog,
  Empty,
  ErrorNote,
  Field,
  Loading,
  PageHeader,
  Panel,
  Select,
  Tag,
  Textarea,
  useToast,
} from "@/components/ui";
import { formatDateTime, zoneLabel } from "@/lib/time";

const STATUS: Record<SwapStatus, { text: string; tone: "neutral" | "signal" | "ok" | "danger" }> = {
  pending: { text: "Waiting for an answer", tone: "signal" },
  accepted: { text: "Accepted", tone: "ok" },
  declined: { text: "Declined", tone: "danger" },
  canceled: { text: "Canceled", tone: "neutral" },
  expired: { text: "Expired", tone: "neutral" },
};

function windowText(start: string, end: string, tz: string) {
  return `${formatDateTime(start, tz)} to ${formatDateTime(end, tz)} ${zoneLabel(tz, new Date(start))}`;
}

const shiftKey = (s: Shift) => `${s.layer}|${s.start}|${s.end}`;

/** Ask a colleague to cover one of your upcoming shifts, optionally taking one of theirs. */
export function RequestSwapDialog({
  open,
  onClose,
  scheduleId: fixedSchedule,
}: {
  open: boolean;
  onClose: () => void;
  scheduleId?: string;
}) {
  const { data: me } = useMe();
  const myId = me?.user.user_id;
  const schedules = useSchedules();
  const members = useMembers();
  const qc = useQueryClient();
  const toast = useToast();
  const [scheduleId, setScheduleId] = useState(fixedSchedule ?? "");
  const [mine, setMine] = useState("");
  const [target, setTarget] = useState("");
  const [exchange, setExchange] = useState("");
  const [reason, setReason] = useState("");
  const schedule = (schedules.data ?? []).find((s) => s.id === scheduleId);
  const tz = schedule?.timezone ?? "UTC";

  // Fixed per open dialog so the query key stays stable while the person chooses.
  const [range] = useState(() => {
    const now = new Date();
    return { from: now.toISOString(), to: new Date(now.getTime() + 90 * 86_400_000).toISOString() };
  });
  const shifts = useQuery({
    queryKey: ["schedules", scheduleId, "assignments", range.from],
    queryFn: () =>
      api.get<Shift[]>(
        `/schedules/${scheduleId}/assignments${query({ from: range.from, to: range.to, limit: 2000 })}`,
      ),
    enabled: open && !!scheduleId,
  });
  const upcoming = useMemo(
    () => (shifts.data ?? []).filter((s) => new Date(s.start) > new Date(range.from)),
    [shifts.data, range.from],
  );
  const myShifts = upcoming.filter((s) => s.user_id === myId);
  const chosen = myShifts.find((s) => shiftKey(s) === mine);
  const theirShifts = chosen
    ? upcoming.filter(
        (s) =>
          s.user_id === target &&
          s.layer === chosen.layer &&
          (s.end <= chosen.start || s.start >= chosen.end),
      )
    : [];
  const offered = theirShifts.find((s) => shiftKey(s) === exchange);

  const create = useMutation({
    mutationFn: () =>
      api.post<SwapRequest>("/swap-requests", {
        schedule_id: scheduleId,
        layer: chosen!.layer,
        target_user_id: target,
        start: chosen!.start,
        end: chosen!.end,
        return_start: offered?.start ?? null,
        return_end: offered?.end ?? null,
        reason,
      }),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: keys.swaps });
      toast("Swap request sent");
      close();
    },
  });

  const reset = () => {
    setScheduleId(fixedSchedule ?? "");
    setMine("");
    setTarget("");
    setExchange("");
    setReason("");
    create.reset();
  };
  const close = () => {
    reset();
    onClose();
  };
  const colleagues = (members.data ?? []).filter((m) => m.user_id !== myId);
  return (
    <Dialog open={open} onClose={close} title="Ask for a swap" wide>
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          if (chosen && target) create.mutate();
        }}
      >
        {!fixedSchedule && (
          <Field label="Schedule" htmlFor="swap-schedule">
            <Select
              id="swap-schedule"
              required
              value={scheduleId}
              onChange={(e) => {
                setScheduleId(e.target.value);
                setMine("");
                setExchange("");
              }}
            >
              <option value="">Choose a schedule</option>
              {(schedules.data ?? [])
                .filter((s) => s.status !== "archived")
                .map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name}
                  </option>
                ))}
            </Select>
          </Field>
        )}
        {scheduleId && shifts.isLoading && <Loading label="Finding your shifts" />}
        {shifts.error && <ErrorNote error={shifts.error} />}
        {scheduleId && shifts.data && myShifts.length === 0 && (
          <p className="text-lg">You have no shifts on this schedule in the next 90 days.</p>
        )}
        {myShifts.length > 0 && (
          <>
            <Field
              label="Your shift"
              htmlFor="swap-mine"
              hint="Only shifts that have not started yet can be swapped."
            >
              <Select
                id="swap-mine"
                required
                value={mine}
                onChange={(e) => {
                  setMine(e.target.value);
                  setExchange("");
                }}
              >
                <option value="">Choose a shift</option>
                {myShifts.map((s) => (
                  <option key={shiftKey(s)} value={shiftKey(s)}>
                    {s.layer === "primary" ? "Primary" : "Secondary"}: {windowText(s.start, s.end, tz)}
                  </option>
                ))}
              </Select>
            </Field>
            <Field label="Who should cover it" htmlFor="swap-target">
              <Select
                id="swap-target"
                required
                value={target}
                onChange={(e) => {
                  setTarget(e.target.value);
                  setExchange("");
                }}
              >
                <option value="">Choose a colleague</option>
                {colleagues.map((m) => (
                  <option key={m.user_id} value={m.user_id}>
                    {m.display_name}
                  </option>
                ))}
              </Select>
            </Field>
            {chosen && target && (
              <Field
                label="In exchange"
                htmlFor="swap-exchange"
                hint="Optional. Offer to take one of their shifts on the same layer."
              >
                <Select id="swap-exchange" value={exchange} onChange={(e) => setExchange(e.target.value)}>
                  <option value="">Nothing, just ask them to cover</option>
                  {theirShifts.map((s) => (
                    <option key={shiftKey(s)} value={shiftKey(s)}>
                      Take their shift: {windowText(s.start, s.end, tz)}
                    </option>
                  ))}
                </Select>
              </Field>
            )}
            <Field
              label="Reason"
              htmlFor="swap-reason"
              hint="Optional. Shown to them and kept with the override."
            >
              <Textarea
                id="swap-reason"
                rows={2}
                maxLength={500}
                value={reason}
                onChange={(e) => setReason(e.target.value)}
              />
            </Field>
          </>
        )}
        {create.error && (
          <p role="alert" className="text-danger">
            {errorMessage(create.error)}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button onClick={close}>Cancel</Button>
          <Button type="submit" variant="primary" disabled={!chosen || !target || create.isPending}>
            Send request
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function SwapItem({ swap, actions }: { swap: SwapRequest; actions?: ReactNode }) {
  const name = useMemberNames();
  const status = STATUS[swap.status];
  return (
    <li className="flex flex-wrap items-start justify-between gap-4">
      <div className="flex min-w-0 flex-col gap-1">
        <p className="text-lg font-semibold">
          {name(swap.requester_id)} asks {name(swap.target_user_id)} to cover
        </p>
        <p className="text-base">
          <Link to={`/schedules/${swap.schedule_id}`} className="underline-offset-4 hover:underline">
            {swap.schedule_name}
          </Link>
          , {swap.layer}: {windowText(swap.start_at, swap.end_at, swap.timezone)}
        </p>
        {swap.return_start_at && swap.return_end_at && (
          <p className="text-base">
            In exchange {name(swap.requester_id)} takes{" "}
            {windowText(swap.return_start_at, swap.return_end_at, swap.timezone)}
          </p>
        )}
        {swap.reason && <p className="text-base text-ink-soft">“{swap.reason}”</p>}
        <div className="mt-1">
          <Tag tone={status.tone}>{status.text}</Tag>
        </div>
      </div>
      {actions && <div className="flex flex-wrap gap-2">{actions}</div>}
    </li>
  );
}

export function SwapsPage() {
  const { data: me } = useMe();
  const can = useCan();
  const myId = me?.user.user_id;
  const [everyone, setEveryone] = useState(false);
  const mineQuery = useSwaps();
  const allQuery = useQuery({
    queryKey: [...keys.swaps, "all"],
    queryFn: () => api.get<SwapRequest[]>("/swap-requests?all=true"),
    enabled: everyone,
  });
  const [requestOpen, setRequestOpen] = useState(false);
  const qc = useQueryClient();
  const toast = useToast();
  const act = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "accept" | "decline" | "cancel" }) =>
      api.post<SwapRequest>(`/swap-requests/${id}/${action}`),
    onSuccess: async (swap) => {
      await qc.invalidateQueries({ queryKey: keys.swaps });
      if (swap.status === "accepted") {
        await qc.invalidateQueries({ queryKey: keys.schedules });
        await qc.invalidateQueries({ queryKey: keys.overview });
      }
      toast(
        swap.status === "accepted" ? "Swap accepted. The schedule is updated." : `Request ${swap.status}`,
      );
    },
    onError: async (e) => {
      toast(errorMessage(e), "danger");
      await qc.invalidateQueries({ queryKey: keys.swaps });
    },
  });

  const list = (everyone ? allQuery.data : mineQuery.data) ?? [];
  const loading = everyone ? allQuery.isLoading : mineQuery.isLoading;
  const error = everyone ? allQuery.error : mineQuery.error;
  const incoming = list.filter((s) => s.status === "pending" && s.target_user_id === myId);
  const outgoing = list.filter((s) => s.status === "pending" && s.target_user_id !== myId);
  const history = list.filter((s) => s.status !== "pending");
  const busy = act.isPending;
  const cancelButton = (s: SwapRequest) =>
    (s.requester_id === myId || can("manage_schedules")) && (
      <Button size="sm" disabled={busy} onClick={() => act.mutate({ id: s.id, action: "cancel" })}>
        Cancel request
      </Button>
    );

  return (
    <div className="flex flex-col gap-6 md:gap-8">
      <PageHeader
        title="Shift swaps"
        lead="Ask a colleague to cover one of your shifts. When they accept, the schedule changes by itself and Discord is told."
        actions={
          <Button variant="primary" onClick={() => setRequestOpen(true)}>
            Ask for a swap
          </Button>
        }
      />
      {can("manage_schedules") && (
        <label className="flex items-center gap-2 text-base">
          <input
            type="checkbox"
            checked={everyone}
            onChange={(e) => setEveryone(e.target.checked)}
            className="size-4 accent-signal"
          />
          Show everyone's requests
        </label>
      )}
      {loading && <Loading />}
      {error && <ErrorNote error={error} />}
      {!loading && !error && list.length === 0 && (
        <Empty title="No swap requests">
          Going on holiday or have a clash? Ask a colleague to cover your shift, or trade one of yours for one
          of theirs.
        </Empty>
      )}
      {incoming.length > 0 && (
        <Panel title="Waiting for you">
          <ul className="flex flex-col gap-6">
            {incoming.map((s) => (
              <SwapItem
                key={s.id}
                swap={s}
                actions={
                  <>
                    <Button
                      variant="primary"
                      size="sm"
                      disabled={busy}
                      onClick={() => act.mutate({ id: s.id, action: "accept" })}
                    >
                      Accept
                    </Button>
                    <Button
                      size="sm"
                      disabled={busy}
                      onClick={() => act.mutate({ id: s.id, action: "decline" })}
                    >
                      Decline
                    </Button>
                  </>
                }
              />
            ))}
          </ul>
        </Panel>
      )}
      {outgoing.length > 0 && (
        <Panel title={everyone ? "Open requests" : "Your requests"}>
          <ul className="flex flex-col gap-6">
            {outgoing.map((s) => (
              <SwapItem key={s.id} swap={s} actions={cancelButton(s)} />
            ))}
          </ul>
        </Panel>
      )}
      {history.length > 0 && (
        <Panel title="Earlier">
          <ul className="flex flex-col gap-6">
            {history.map((s) => (
              <SwapItem key={s.id} swap={s} />
            ))}
          </ul>
        </Panel>
      )}
      <RequestSwapDialog open={requestOpen} onClose={() => setRequestOpen(false)} />
    </div>
  );
}
