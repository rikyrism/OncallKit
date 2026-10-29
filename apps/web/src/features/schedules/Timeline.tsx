import { useMemo } from "react";
import type { Layer, Shift } from "@/lib/types";
import { cx } from "@/components/ui";
import { formatDateTime, formatDay, formatDayShort, localDateKey } from "@/lib/time";

/** A stable tone per person, from a small set of steps on the ink scale plus the signal. */
const TONES = ["bg-signal text-signal-ink", "bg-ink text-ground", "bg-surface-3 text-ink", "bg-ink-soft text-ground"];

export function toneFor(userId: string, order: string[]): string {
  const i = Math.max(0, order.indexOf(userId));
  return TONES[i % TONES.length]!;
}

/**
 * Horizontal timeline: one row per layer, each shift a block sized by its duration.
 * Days are marked by alternating background tone rather than grid lines.
 */
export function Timeline({
  shifts,
  from,
  to,
  timezone,
  name,
  now,
}: {
  shifts: Shift[];
  from: Date;
  to: Date;
  timezone: string;
  name: (id: string) => string;
  now?: Date;
}) {
  const span = to.getTime() - from.getTime();
  const pct = (t: number) => `${((Math.min(Math.max(t, from.getTime()), to.getTime()) - from.getTime()) / span) * 100}%`;
  const order = useMemo(() => Array.from(new Set(shifts.map((s) => s.user_id))), [shifts]);

  // One band per local day; over three weeks, one band per week so labels stay readable.
  const days = useMemo(() => {
    const out: { key: string; start: number; label: string }[] = [];
    let last = "";
    for (let t = from.getTime(); t < to.getTime(); t += 3_600_000) {
      const key = localDateKey(new Date(t), timezone);
      if (key !== last) {
        out.push({ key, start: t, label: "" });
        last = key;
      }
    }
    const step = out.length > 21 ? 7 : 1;
    return out
      .filter((_, i) => i % step === 0)
      .map((d) => ({ ...d, label: (step === 1 ? formatDayShort : formatDay)(new Date(d.start).toISOString(), timezone) }));
  }, [from, to, timezone]);

  const layers: Layer[] = shifts.some((s) => s.layer === "secondary") ? ["primary", "secondary"] : ["primary"];

  return (
    <div className="overflow-x-auto">
      <div className="relative min-w-[720px]">
        <div className="relative flex h-8 text-sm">
          {days.map((d, i) => {
            const next = days[i + 1]?.start ?? to.getTime();
            return (
              <div key={d.key} className={cx("absolute inset-y-0 overflow-hidden whitespace-nowrap px-1 pt-1 text-xs md:text-sm", i % 2 ? "bg-surface-2" : "bg-surface")} style={{ left: pct(d.start), width: `calc(${pct(next)} - ${pct(d.start)})` }}>
                {d.label}
              </div>
            );
          })}
        </div>
        {layers.map((layer) => (
          <div key={layer} className="mt-1.5">
            <p className="mb-1 text-sm font-medium">{layer === "primary" ? "Primary" : "Secondary"}</p>
            <div className="relative h-12 bg-surface-2">
              {shifts
                .filter((s) => s.layer === layer)
                .map((s) => {
                  const start = new Date(s.start).getTime();
                  const end = new Date(s.end).getTime();
                  return (
                    <div
                      key={`${s.start}-${s.user_id}`}
                      title={`${name(s.user_id)}: ${formatDateTime(s.start, timezone)} to ${formatDateTime(s.end, timezone)}${s.source.kind === "override" ? " (override)" : ""}`}
                      className={cx("absolute inset-y-0 flex items-center overflow-hidden px-2 text-sm font-medium", toneFor(s.user_id, order), s.source.kind === "override" && "outline-2 -outline-offset-2 outline-dashed outline-current")}
                      style={{ left: pct(start), width: `calc(${pct(end)} - ${pct(start)} - 2px)` }}
                    >
                      <span className="truncate">{name(s.user_id)}</span>
                    </div>
                  );
                })}
            </div>
          </div>
        ))}
        {now && now > from && now < to && <div aria-hidden className="pointer-events-none absolute inset-y-0 w-0.5 bg-danger" style={{ left: pct(now.getTime()) }} />}
      </div>
    </div>
  );
}
