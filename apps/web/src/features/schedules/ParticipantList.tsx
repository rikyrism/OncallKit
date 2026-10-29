import { ArrowDown, ArrowUp, X } from "lucide-react";
import type { Member } from "@/lib/types";
import { Select } from "@/components/ui";

/** An ordered rotation: people take turns in this order, then start again from the top. */
export function ParticipantList({
  id,
  value,
  onChange,
  members,
  invalid,
}: {
  id: string;
  value: string[];
  onChange: (next: string[]) => void;
  members: Member[];
  invalid?: boolean;
}) {
  const name = (uid: string) => members.find((m) => m.user_id === uid)?.display_name ?? "Former member";
  const available = members.filter((m) => !value.includes(m.user_id));
  const move = (i: number, d: -1 | 1) => {
    const next = [...value];
    const j = i + d;
    if (j < 0 || j >= next.length) return;
    [next[i], next[j]] = [next[j]!, next[i]!];
    onChange(next);
  };
  return (
    <div className="flex flex-col gap-2">
      {value.length > 0 && (
        <ol className="flex flex-col gap-1" aria-label="Rotation order">
          {value.map((uid, i) => (
            <li key={uid} className="flex items-center gap-3 bg-surface-2 py-1.5 pl-3 pr-1.5">
              <span className="w-6 text-right text-sm tabular text-ink-soft" aria-hidden>
                {i + 1}.
              </span>
              <span className="flex-1 font-medium">{name(uid)}</span>
              <button type="button" className="p-2 hover:bg-surface-3 disabled:opacity-30" onClick={() => move(i, -1)} disabled={i === 0} aria-label={`Move ${name(uid)} earlier`}>
                <ArrowUp className="size-4" />
              </button>
              <button type="button" className="p-2 hover:bg-surface-3 disabled:opacity-30" onClick={() => move(i, 1)} disabled={i === value.length - 1} aria-label={`Move ${name(uid)} later`}>
                <ArrowDown className="size-4" />
              </button>
              <button type="button" className="p-2 hover:bg-surface-3" onClick={() => onChange(value.filter((v) => v !== uid))} aria-label={`Remove ${name(uid)}`}>
                <X className="size-4" />
              </button>
            </li>
          ))}
        </ol>
      )}
      <Select
        id={id}
        aria-invalid={invalid}
        value=""
        onChange={(e) => e.target.value && onChange([...value, e.target.value])}
        disabled={available.length === 0}
      >
        <option value="">{available.length ? "Add a person…" : "Everyone is in this rotation"}</option>
        {available.map((m) => (
          <option key={m.user_id} value={m.user_id}>
            {m.display_name} ({m.email})
          </option>
        ))}
      </Select>
    </div>
  );
}
