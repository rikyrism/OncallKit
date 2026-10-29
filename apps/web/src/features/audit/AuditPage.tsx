import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, query } from "@/lib/api";
import { useCan } from "@/lib/queries";
import type { AuditEntry, Page } from "@/lib/types";
import { Button, Empty, ErrorNote, Loading, PageHeader, Select } from "@/components/ui";
import { browserTimezone, formatDateTime } from "@/lib/time";

const GROUPS = ["schedule", "override", "team", "member", "invitation", "integration", "organization", "delivery"];

function describe(e: AuditEntry): string {
  const words = e.action.replace(/[._]/g, " ");
  const meta = e.metadata as Record<string, string | undefined>;
  const detail = meta.name ?? meta.email ?? (meta.from && meta.to ? `${meta.from} → ${meta.to}` : undefined);
  return detail ? `${words}: ${detail}` : words;
}

export function AuditPage() {
  const can = useCan();
  const [action, setAction] = useState("");
  const [offset, setOffset] = useState(0);
  const tz = browserTimezone();
  const q = useQuery({
    queryKey: ["audit", action, offset],
    queryFn: () => api.get<Page<AuditEntry>>(`/audit-logs${query({ action: action ? `${action}.` : "", limit: 50, offset })}`),
    enabled: can("view_audit_logs"),
  });
  if (!can("view_audit_logs")) return <ErrorNote error={new Error("Only administrators can see the audit log.")} />;
  return (
    <div>
      <PageHeader title="Audit logs" lead="Who changed what, and when. Entries cannot be edited." />
      <label className="mb-6 flex max-w-xs flex-col gap-1.5 text-base font-medium">
        Kind of change
        <Select value={action} onChange={(e) => (setAction(e.target.value), setOffset(0))}>
          <option value="">Everything</option>
          {GROUPS.map((g) => (
            <option key={g} value={g}>
              {g}
            </option>
          ))}
        </Select>
      </label>
      {q.isLoading && <Loading />}
      {q.error && <ErrorNote error={q.error} />}
      {q.data?.items.length === 0 && <Empty title="No entries" />}
      {q.data && q.data.items.length > 0 && (
        <div className="overflow-x-auto bg-surface">
          <table className="data-table">
            <thead>
              <tr>
                <th>When</th>
                <th>Who</th>
                <th>What</th>
              </tr>
            </thead>
            <tbody>
              {q.data.items.map((e) => (
                <tr key={e.id}>
                  <td className="whitespace-nowrap tabular">{formatDateTime(e.created_at, tz)}</td>
                  <td>{e.actor_name ?? "System"}</td>
                  <td className="first-letter:uppercase">{describe(e)}</td>
                </tr>
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
