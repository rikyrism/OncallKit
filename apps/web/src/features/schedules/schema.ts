import { z } from "zod";

export const REMINDER_PRESETS = [
  { minutes: 10080, label: "1 week before" },
  { minutes: 1440, label: "24 hours before" },
  { minutes: 60, label: "1 hour before" },
  { minutes: 15, label: "15 minutes before" },
];

const localDateTime = z.string().regex(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/, "Choose a date and time");

export const scheduleSchema = z
  .object({
    name: z.string().trim().min(1, "Name the schedule").max(120, "Use at most 120 characters"),
    description: z.string().max(2000),
    team_id: z.string(),
    timezone: z.string().min(1, "Choose a timezone"),
    start_local: localDateTime,
    open_ended: z.boolean(),
    end_local: z.string(),
    interval: z.coerce.number<number>().int("Use a whole number").min(1, "At least 1").max(365, "At most 365"),
    unit: z.enum(["hours", "days", "weeks"]),
    primary: z.array(z.string()).min(1, "Add at least one person to the primary rotation"),
    secondary: z.array(z.string()),
    reminder_offsets_minutes: z.array(z.number()).max(5),
    notify_on_boundary: z.boolean(),
  })
  .superRefine((v, ctx) => {
    if (!v.open_ended) {
      if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(v.end_local)) {
        ctx.addIssue({ code: "custom", path: ["end_local"], message: "Choose an end, or make the schedule open-ended" });
      } else if (v.end_local <= v.start_local) {
        ctx.addIssue({ code: "custom", path: ["end_local"], message: "The end must be after the start" });
      }
    }
    if (v.unit === "weeks" && v.interval > 52) ctx.addIssue({ code: "custom", path: ["interval"], message: "At most 52 weeks" });
  });

export type ScheduleForm = z.infer<typeof scheduleSchema>;

export function toPayload(v: ScheduleForm) {
  return {
    name: v.name.trim(),
    description: v.description.trim(),
    team_id: v.team_id || null,
    timezone: v.timezone,
    start_local: v.start_local,
    end_local: v.open_ended ? null : v.end_local,
    rotation: { interval: v.interval, unit: v.unit, primary: v.primary, secondary: v.secondary },
    reminder_offsets_minutes: v.reminder_offsets_minutes,
    notify_on_boundary: v.notify_on_boundary,
  };
}
