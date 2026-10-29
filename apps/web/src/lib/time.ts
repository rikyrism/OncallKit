// All instants from the API are UTC. Display always happens in an explicit IANA timezone,
// normally the schedule's own, so "09:00" means the same thing to everyone reading it.

const cache = new Map<string, Intl.DateTimeFormat>();

function fmt(timeZone: string, opts: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = timeZone + JSON.stringify(opts);
  let f = cache.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat("en-GB", { timeZone, ...opts });
    cache.set(key, f);
  }
  return f;
}

export function formatDateTime(iso: string, timeZone: string): string {
  return fmt(timeZone, { day: "numeric", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(new Date(iso));
}

export function formatLong(iso: string, timeZone: string): string {
  return fmt(timeZone, {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(new Date(iso));
}

export function formatDay(iso: string, timeZone: string): string {
  return fmt(timeZone, { weekday: "short", day: "numeric", month: "short" }).format(new Date(iso));
}

/** "Wed 28": compact enough for a timeline column. */
export function formatDayShort(iso: string, timeZone: string): string {
  return fmt(timeZone, { weekday: "short", day: "numeric" }).format(new Date(iso));
}

export function formatTime(iso: string, timeZone: string): string {
  return fmt(timeZone, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(new Date(iso));
}

export function zoneLabel(timeZone: string, at: Date = new Date()): string {
  const part = fmt(timeZone, { timeZoneName: "short" }).formatToParts(at).find((p) => p.type === "timeZoneName");
  return part?.value ?? timeZone;
}

/** Calendar date (YYYY-MM-DD) of an instant in a timezone. */
export function localDateKey(iso: string | Date, timeZone: string): string {
  const parts = fmt(timeZone, { year: "numeric", month: "2-digit", day: "2-digit" }).formatToParts(typeof iso === "string" ? new Date(iso) : iso);
  const get = (t: string) => parts.find((p) => p.type === t)?.value ?? "";
  return `${get("year")}-${get("month")}-${get("day")}`;
}

/** Local wall-clock "YYYY-MM-DDTHH:MM" of an instant in a timezone (for datetime-local inputs). */
export function toLocalInput(iso: string | Date, timeZone: string): string {
  const parts = fmt(timeZone, { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).formatToParts(
    typeof iso === "string" ? new Date(iso) : iso,
  );
  const get = (t: string) => parts.find((p) => p.type === t)?.value ?? "00";
  return `${get("year")}-${get("month")}-${get("day")}T${get("hour")}:${get("minute")}`;
}

export function relative(iso: string, now: Date = new Date()): string {
  const diff = new Date(iso).getTime() - now.getTime();
  const abs = Math.abs(diff);
  const mins = Math.round(abs / 60000);
  let text: string;
  if (mins < 1) text = "less than a minute";
  else if (mins < 60) text = `${mins} min`;
  else if (mins < 60 * 48) text = `${Math.round(mins / 60)} h`;
  else text = `${Math.round(mins / 1440)} days`;
  return diff >= 0 ? `in ${text}` : `${text} ago`;
}

export function durationText(startIso: string, endIso: string): string {
  const hours = (new Date(endIso).getTime() - new Date(startIso).getTime()) / 3_600_000;
  if (hours < 48) return `${Math.round(hours)} h`;
  return `${Math.round(hours / 24)} days`;
}

export const COMMON_TIMEZONES = [
  "UTC",
  "Asia/Jakarta",
  "Asia/Makassar",
  "Asia/Jayapura",
  "Asia/Singapore",
  "Asia/Kolkata",
  "Asia/Tokyo",
  "Australia/Sydney",
  "Europe/London",
  "Europe/Berlin",
  "America/New_York",
  "America/Chicago",
  "America/Los_Angeles",
  "America/Sao_Paulo",
];

export function allTimezones(): string[] {
  const supported = (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf;
  const list = supported ? supported("timeZone") : COMMON_TIMEZONES;
  return Array.from(new Set(["UTC", ...list]));
}

export function browserTimezone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
}
