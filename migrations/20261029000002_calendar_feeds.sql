-- Secret iCalendar subscription links. Calendar apps cannot send cookies, so each feed has its
-- own random token; only its SHA-256 hash is stored. A feed covers either the owner's own shifts
-- across the organization (schedule_id NULL) or every shift of one schedule.
CREATE TABLE calendar_feeds (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id          uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    organization_id  uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    schedule_id      uuid REFERENCES schedules(id) ON DELETE CASCADE,
    token_hash       bytea NOT NULL UNIQUE,
    created_at       timestamptz NOT NULL DEFAULT now(),
    last_used_at     timestamptz
);

CREATE INDEX calendar_feeds_owner_idx ON calendar_feeds (user_id, organization_id);
