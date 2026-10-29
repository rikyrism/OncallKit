import { describe, expect, it } from "vitest";
import { scheduleSchema, toPayload, type ScheduleForm } from "@/features/schedules/schema";
import { validateWebhook } from "@/features/integrations/IntegrationsPage";
import { localToUtc } from "@/features/schedules/ScheduleDetailPage";
import { statusText } from "@/features/notifications/NotificationsPage";
import { formatDateTime, localDateKey, toLocalInput } from "@/lib/time";

const base: ScheduleForm = {
  name: "DevOps On-call",
  description: "",
  team_id: "",
  timezone: "Asia/Jakarta",
  start_local: "2026-11-03T09:00",
  open_ended: false,
  end_local: "2027-02-03T09:00",
  interval: 7,
  unit: "days",
  primary: ["a", "b", "c"],
  secondary: [],
  reminder_offsets_minutes: [1440, 60],
  notify_on_boundary: true,
};

describe("schedule form validation", () => {
  it("accepts a valid weekly schedule", () => {
    expect(scheduleSchema.safeParse(base).success).toBe(true);
  });

  it("rejects an end before the start", () => {
    const r = scheduleSchema.safeParse({ ...base, end_local: "2026-11-01T09:00" });
    expect(r.success).toBe(false);
    expect(r.error?.issues[0]?.path).toEqual(["end_local"]);
  });

  it("requires an end unless open-ended", () => {
    expect(scheduleSchema.safeParse({ ...base, end_local: "" }).success).toBe(false);
    expect(scheduleSchema.safeParse({ ...base, end_local: "", open_ended: true }).success).toBe(true);
  });

  it("rejects an empty primary rotation", () => {
    const r = scheduleSchema.safeParse({ ...base, primary: [] });
    expect(r.error?.issues[0]?.message).toMatch(/at least one person/);
  });

  it("rejects a zero interval", () => {
    expect(scheduleSchema.safeParse({ ...base, interval: 0 }).success).toBe(false);
  });

  it("builds the API payload", () => {
    const p = toPayload({ ...base, open_ended: true });
    expect(p.end_local).toBeNull();
    expect(p.team_id).toBeNull();
    expect(p.rotation).toEqual({ interval: 7, unit: "days", primary: ["a", "b", "c"], secondary: [] });
  });
});

describe("webhook validation", () => {
  it("accepts Discord webhooks only", () => {
    expect(validateWebhook("https://discord.com/api/webhooks/123/abcdefghijklmnopqrstuvwxyz")).toBeNull();
    expect(validateWebhook("https://discord.com/api/v10/webhooks/123/abcdefghijklmnopqrstuvwxyz")).toBeNull();
    expect(validateWebhook("http://discord.com/api/webhooks/123/abcdefghijklmnopqrstuvwxyz")).not.toBeNull();
    expect(validateWebhook("https://discord.com.evil.io/api/webhooks/123/abcdefghijklmnopqrstuvwxyz")).not.toBeNull();
    expect(validateWebhook("")).not.toBeNull();
  });
});

describe("time handling", () => {
  it("shows instants in the schedule's timezone", () => {
    expect(formatDateTime("2026-11-03T02:00:00Z", "Asia/Jakarta")).toBe("3 Nov 2026, 09:00");
    expect(localDateKey("2026-11-02T20:00:00Z", "Asia/Jakarta")).toBe("2026-11-03");
    expect(toLocalInput("2026-11-03T02:00:00Z", "Asia/Jakarta")).toBe("2026-11-03T09:00");
  });

  it("converts wall-clock time to UTC, including across DST", () => {
    expect(localToUtc("2026-11-03T09:00", "Asia/Jakarta")).toBe("2026-11-03T02:00:00Z");
    expect(localToUtc("2026-07-01T09:00", "America/New_York")).toBe("2026-07-01T13:00:00Z");
    expect(localToUtc("2026-12-01T09:00", "America/New_York")).toBe("2026-12-01T14:00:00Z");
    expect(localToUtc("nonsense", "UTC")).toBeNull();
  });
});

describe("delivery status", () => {
  it("never calls a waiting delivery delivered", () => {
    expect(statusText({ status: "pending", attempts: 0, next_attempt_at: new Date().toISOString() })).toBe("Queued");
    expect(statusText({ status: "pending", attempts: 2, next_attempt_at: new Date(Date.now() + 600_000).toISOString() })).toMatch(/^Retry in/);
    expect(statusText({ status: "failed", attempts: 6, next_attempt_at: new Date().toISOString() })).toBe("Failed");
    expect(statusText({ status: "succeeded", attempts: 1, next_attempt_at: new Date().toISOString() })).toBe("Delivered");
  });
});
