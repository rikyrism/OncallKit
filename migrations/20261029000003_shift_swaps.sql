-- Shift swap requests between members. The requester asks the target to cover [start_at, end_at)
-- on one layer, optionally taking [return_start_at, return_end_at) of the target's in exchange.
-- Accepting turns the request into ordinary overrides; the request keeps the history.
CREATE TABLE shift_swaps (
    id                uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id   uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    schedule_id       uuid NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    layer             text NOT NULL CHECK (layer IN ('primary', 'secondary')),
    requester_id      uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    start_at          timestamptz NOT NULL,
    end_at            timestamptz NOT NULL,
    return_start_at   timestamptz,
    return_end_at     timestamptz,
    reason            text NOT NULL DEFAULT '' CHECK (length(reason) <= 500),
    status            text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'accepted', 'declined', 'canceled')),
    created_at        timestamptz NOT NULL DEFAULT now(),
    decided_at        timestamptz,
    decided_by        uuid REFERENCES users(id) ON DELETE SET NULL,
    CHECK (end_at > start_at),
    CHECK ((return_start_at IS NULL) = (return_end_at IS NULL)),
    CHECK (return_end_at IS NULL OR return_end_at > return_start_at),
    CHECK (requester_id <> target_user_id)
);

CREATE INDEX shift_swaps_org_status_idx ON shift_swaps (organization_id, status);
CREATE INDEX shift_swaps_people_idx ON shift_swaps (requester_id, target_user_id);
