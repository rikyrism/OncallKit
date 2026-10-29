import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import type { CalendarFeed, CreatedCalendarFeed } from "@/lib/types";
import { Button, buttonClass, ConfirmDialog, Dialog, Input, Loading, Panel, useToast } from "@/components/ui";
import { browserTimezone, formatDateTime, relative } from "@/lib/time";

const FEEDS_KEY = ["calendar-feeds"] as const;

/**
 * Creates a secret calendar link and shows it once. Without a schedule the link covers the
 * signed-in person's own shifts across the organization.
 */
export function SubscribeDialog({
  open,
  onClose,
  scheduleId,
  scheduleName,
}: {
  open: boolean;
  onClose: () => void;
  scheduleId?: string;
  scheduleName?: string;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  const [created, setCreated] = useState<CreatedCalendarFeed | null>(null);
  const create = useMutation({
    mutationFn: () => api.post<CreatedCalendarFeed>("/calendar-feeds", { schedule_id: scheduleId ?? null }),
    onSuccess: async (feed) => {
      setCreated(feed);
      await qc.invalidateQueries({ queryKey: FEEDS_KEY });
    },
  });
  const close = () => {
    setCreated(null);
    create.reset();
    onClose();
  };
  const what = scheduleName ? `every shift of ${scheduleName}` : "your own shifts in every schedule";
  return (
    <Dialog open={open} onClose={close} title="Add to your calendar">
      {created ? (
        <div className="flex flex-col gap-4">
          <p className="text-lg">
            This link is shown once. Your calendar app checks it every hour or so, so changes to the schedule
            show up by themselves.
          </p>
          <Input
            readOnly
            value={created.url}
            onFocus={(e) => e.currentTarget.select()}
            aria-label="Calendar link"
          />
          <ul className="flex flex-col gap-2 text-base">
            <li>
              <strong>Google Calendar:</strong> Other calendars, +, From URL, then paste the link.
            </li>
            <li>
              <strong>Apple Calendar or Outlook:</strong> use Open in calendar app below.
            </li>
          </ul>
          <p className="text-sm text-ink-soft">
            Anyone who has the link can see these shifts. Revoke it on your profile page if it leaks.
          </p>
          <div className="flex flex-wrap justify-end gap-2">
            <Button
              variant="primary"
              onClick={async () => {
                await navigator.clipboard.writeText(created.url).catch(() => {});
                toast("Link copied");
              }}
            >
              Copy link
            </Button>
            <a href={created.webcal_url} className={buttonClass()}>
              Open in calendar app
            </a>
            <Button onClick={close}>Done</Button>
          </div>
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          <p className="text-lg">
            Get a private link that shows {what} in Google Calendar, Apple Calendar or Outlook, from 30 days
            ago to 6 months ahead.
          </p>
          {create.error && (
            <p role="alert" className="text-danger">
              {errorMessage(create.error)}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button onClick={close}>Cancel</Button>
            <Button variant="primary" onClick={() => create.mutate()} disabled={create.isPending}>
              Create link
            </Button>
          </div>
        </div>
      )}
    </Dialog>
  );
}

/** The signed-in person's calendar links, for the profile page. */
export function CalendarFeedsPanel() {
  const qc = useQueryClient();
  const toast = useToast();
  const feeds = useQuery({ queryKey: FEEDS_KEY, queryFn: () => api.get<CalendarFeed[]>("/calendar-feeds") });
  const [subscribeOpen, setSubscribeOpen] = useState(false);
  const [revoking, setRevoking] = useState<CalendarFeed | null>(null);
  const revoke = useMutation({
    mutationFn: (id: string) => api.delete(`/calendar-feeds/${id}`),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: FEEDS_KEY });
      setRevoking(null);
      toast("Calendar link revoked");
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });
  const tz = browserTimezone();
  return (
    <Panel
      title="Calendar"
      actions={
        <Button size="sm" onClick={() => setSubscribeOpen(true)}>
          Add my shifts to a calendar
        </Button>
      }
    >
      {feeds.isLoading && <Loading />}
      {feeds.data && feeds.data.length === 0 && (
        <p>No calendar links yet. Add your shifts to see them next to your meetings.</p>
      )}
      <ul className="flex flex-col gap-4">
        {(feeds.data ?? []).map((f) => (
          <li key={f.id} className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <p className="font-medium">
                {f.schedule_name ? `All shifts of ${f.schedule_name}` : "My shifts"}
              </p>
              <p className="text-sm text-ink-soft">
                Created {formatDateTime(f.created_at, tz)} ·{" "}
                {f.last_used_at ? `last read ${relative(f.last_used_at)}` : "not read yet"}
              </p>
            </div>
            <Button size="sm" onClick={() => setRevoking(f)}>
              Revoke
            </Button>
          </li>
        ))}
      </ul>
      <SubscribeDialog open={subscribeOpen} onClose={() => setSubscribeOpen(false)} />
      <ConfirmDialog
        open={!!revoking}
        title="Revoke this calendar link?"
        confirmLabel="Revoke"
        busy={revoke.isPending}
        onConfirm={() => revoking && revoke.mutate(revoking.id)}
        onClose={() => setRevoking(null)}
      >
        Calendars subscribed to it stop updating and will show the shifts as removed.
      </ConfirmDialog>
    </Panel>
  );
}
