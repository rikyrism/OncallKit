import { Fragment, useState } from "react";
import { Link } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage, query } from "@/lib/api";
import { useCan, useIntegrations } from "@/lib/queries";
import type { Attempt, Delivery, DeliveryStatus, Page } from "@/lib/types";
import { Button, Empty, ErrorNote, Loading, PageHeader, Select, Tag, useToast } from "@/components/ui";
import { browserTimezone, formatDateTime, relative } from "@/lib/time";
import { EVENT_LABELS } from "@/features/integrations/IntegrationsPage";

export const STATUS: Record<DeliveryStatus, { label: string; tone: "neutral" | "ok" | "danger" | "signal" }> = {
  succeeded: { label: "Delivered", tone: "ok" },
  failed: { label: "Failed", tone: "danger" },
  pending: { label: "Waiting", tone: "signal" },
  sending: { label: "Sending", tone: "signal" },
  canceled: { label: "Canceled", tone: "neutral" },
};

export function statusText(d: Pick<Delivery, "status" | "attempts" | "next_attempt_at">): string {
  if (d.status === "pending" && d.attempts > 0) return `Retry ${relative(d.next_attempt_at)}`;
  if (d.status === "pending") return "Queued";
  return STATUS[d.status].label;
}

function Attempts({ id }: { id: string }) {
  const q = useQuery({ queryKey: ["attempts", id], queryFn: () => api.get<Attempt[]>(`/notifications/deliveries/${id}/attempts`) });
  const tz = browserTimezone();
  if (q.isLoading) return <Loading />;
  if (!q.data?.length) return <p>No attempts recorded yet.</p>;
  return (
    <ol className="flex flex-col gap-1 text-base">
      {q.data.map((a) => (
        <li key={a.attempt}>
          Attempt {a.attempt}, {formatDateTime(a.started_at, tz)}: {a.success ? "accepted by Discord" : (a.error ?? "failed")}
          {a.status_code ? ` (HTTP ${a.status_code})` : ""}
        </li>
      ))}
    </ol>
  );
}

export function NotificationsPage() {
  const can = useCan();
  const integrations = useIntegrations(can("view_integrations"));
  const [status, setStatus] = useState("all");
  const [integration, setIntegration] = useState("");
  const [offset, setOffset] = useState(0);
  const [open, setOpen] = useState<string | null>(null);
  const qc = useQueryClient();
  const toast = useToast();
  const tz = browserTimezone();
  const q = useQuery({
    queryKey: ["deliveries", status, integration, offset],
    queryFn: () => api.get<Page<Delivery>>(`/notifications/deliveries${query({ status, integration_id: integration, limit: 50, offset })}`),
    refetchInterval: 15_000,
  });
  const retry = useMutation({
    mutationFn: (id: string) => api.post(`/notifications/deliveries/${id}/retry`),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: ["deliveries"] });
      toast("Queued for another attempt");
    },
    onError: (e) => toast(errorMessage(e), "danger"),
  });

  return (
    <div>
      <PageHeader title="Notification history" lead="Every message OncallKit tried to send, and exactly what Discord answered. Delivered means Discord accepted it, not that someone read it." />
      <div className="mb-6 flex flex-wrap gap-4">
        <label className="flex flex-col gap-1.5 text-base font-medium">
          Status
          <Select className="w-48" value={status} onChange={(e) => (setStatus(e.target.value), setOffset(0))}>
            <option value="all">All</option>
            <option value="succeeded">Delivered</option>
            <option value="failed">Failed</option>
            <option value="pending">Waiting or retrying</option>
          </Select>
        </label>
        <label className="flex flex-col gap-1.5 text-base font-medium">
          Channel
          <Select className="w-56" value={integration} onChange={(e) => (setIntegration(e.target.value), setOffset(0))}>
            <option value="">All channels</option>
            {(integrations.data ?? []).map((i) => (
              <option key={i.id} value={i.id}>
                {i.name}
              </option>
            ))}
          </Select>
        </label>
      </div>
      {q.isLoading && <Loading />}
      {q.error && <ErrorNote error={q.error} retry={() => q.refetch()} />}
      {q.data?.items.length === 0 && (
        <Empty title="Nothing sent yet">
          Deliveries appear here once a schedule hands over or a reminder is due. Use <Link to="/integrations" className="underline underline-offset-4">Send test</Link> to check a channel now.
        </Empty>
      )}
      {q.data && q.data.items.length > 0 && (
        <div className="overflow-x-auto bg-surface">
          <table className="data-table">
            <thead>
              <tr>
                <th>When</th>
                <th>Event</th>
                <th>Channel</th>
                <th>Status</th>
                <th>Attempts</th>
                <th>
                  <span className="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {q.data.items.map((d) => (
                <Fragment key={d.id}>
                  <tr>
                    <td className="tabular">{formatDateTime(d.created_at, tz)}</td>
                    <td>
                      <span className="block font-medium">{EVENT_LABELS[d.event_type] ?? (d.event_type === "test" ? "Test message" : d.event_type)}</span>
                      {d.schedule_name && d.schedule_id && (
                        <Link to={`/schedules/${d.schedule_id}`} className="text-sm underline-offset-4 hover:underline">
                          {d.schedule_name}
                        </Link>
                      )}
                    </td>
                    <td>{d.integration_name}</td>
                    <td>
                      <Tag tone={STATUS[d.status].tone}>{statusText(d)}</Tag>
                      {d.last_error && d.status !== "succeeded" && <span className="mt-1 block max-w-xs text-sm text-ink-soft">{d.last_error}</span>}
                    </td>
                    <td className="tabular">
                      {d.attempts} of {d.max_attempts}
                    </td>
                    <td className="whitespace-nowrap text-right">
                      <Button size="sm" variant="ghost" aria-expanded={open === d.id} onClick={() => setOpen(open === d.id ? null : d.id)}>
                        Details
                      </Button>
                      {d.status === "failed" && can("manage_integrations") && (
                        <Button size="sm" onClick={() => retry.mutate(d.id)}>
                          Retry
                        </Button>
                      )}
                    </td>
                  </tr>
                  {open === d.id && (
                    <tr>
                      <td colSpan={6} className="bg-surface-2">
                        <Attempts id={d.id} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {q.data && (offset > 0 || q.data.has_more) && (
        <div className="mt-4 flex gap-2">
          <Button size="sm" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - 50))}>
            Newer
          </Button>
          <Button size="sm" disabled={!q.data.has_more} onClick={() => setOffset(offset + 50)}>
            Older
          </Button>
        </div>
      )}
    </div>
  );
}
