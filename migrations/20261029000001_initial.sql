-- OncallKit initial schema.
-- All timestamps are timestamptz and stored in UTC. Schedules keep their IANA timezone
-- separately so calendar arithmetic (daily/weekly rotations) is done in local time.

CREATE TABLE users (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email              text NOT NULL,
    display_name       text NOT NULL,
    avatar_url         text,
    discord_user_id    text CHECK (discord_user_id ~ '^[0-9]{5,25}$'),
    is_instance_admin  boolean NOT NULL DEFAULT false,
    disabled_at        timestamptz,
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX users_email_idx ON users (lower(email));

-- External identities. The provider subject is the stable identity key; email may change.
CREATE TABLE identity_accounts (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider       text NOT NULL,
    subject        text NOT NULL,
    email          text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    last_login_at  timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider, subject)
);
CREATE INDEX identity_accounts_user_idx ON identity_accounts (user_id);

-- Emails pre-authorised as instance administrators through the CLI bootstrap command.
CREATE TABLE instance_admin_grants (
    email        text PRIMARY KEY,
    created_at   timestamptz NOT NULL DEFAULT now(),
    consumed_at  timestamptz
);

-- In-flight OAuth/OIDC authorisation requests. Bound to the browser by binding_hash.
CREATE TABLE oauth_flows (
    id             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    provider       text NOT NULL,
    state_hash     bytea NOT NULL UNIQUE,
    binding_hash   bytea NOT NULL,
    nonce          text NOT NULL,
    pkce_verifier  text NOT NULL,
    return_to      text NOT NULL DEFAULT '/',
    created_at     timestamptz NOT NULL DEFAULT now(),
    expires_at     timestamptz NOT NULL
);
CREATE INDEX oauth_flows_expires_idx ON oauth_flows (expires_at);

CREATE TABLE organizations (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    slug        text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9][a-z0-9-]{0,62}$'),
    timezone    text NOT NULL DEFAULT 'UTC',
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    id                      uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    token_hash              bytea NOT NULL UNIQUE,
    user_id                 uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    csrf_token              text NOT NULL,
    active_organization_id  uuid REFERENCES organizations(id) ON DELETE SET NULL,
    user_agent              text,
    ip_address              text,
    created_at              timestamptz NOT NULL DEFAULT now(),
    last_seen_at            timestamptz NOT NULL DEFAULT now(),
    expires_at              timestamptz NOT NULL,
    revoked_at              timestamptz
);
CREATE INDEX sessions_user_idx ON sessions (user_id) WHERE revoked_at IS NULL;

CREATE TABLE organization_memberships (
    organization_id  uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id          uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role             text NOT NULL CHECK (role IN ('owner', 'admin', 'scheduler', 'member')),
    created_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organization_id, user_id)
);
CREATE INDEX organization_memberships_user_idx ON organization_memberships (user_id);

CREATE TABLE invitations (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id  uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    email            text NOT NULL,
    role             text NOT NULL CHECK (role IN ('admin', 'scheduler', 'member')),
    token_hash       bytea NOT NULL UNIQUE,
    invited_by       uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    expires_at       timestamptz NOT NULL,
    accepted_at      timestamptz,
    accepted_by      uuid REFERENCES users(id) ON DELETE SET NULL,
    revoked_at       timestamptz
);
CREATE INDEX invitations_org_idx ON invitations (organization_id, created_at DESC);
CREATE INDEX invitations_email_idx ON invitations (lower(email)) WHERE accepted_at IS NULL AND revoked_at IS NULL;

CREATE TABLE teams (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id  uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name             text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    description      text NOT NULL DEFAULT '',
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX teams_org_name_idx ON teams (organization_id, lower(name));

CREATE TABLE team_memberships (
    team_id     uuid NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    user_id     uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (team_id, user_id)
);

CREATE TABLE schedules (
    id                       uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id          uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    team_id                  uuid REFERENCES teams(id) ON DELETE SET NULL,
    name                     text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    description              text NOT NULL DEFAULT '',
    timezone                 text NOT NULL,
    start_at                 timestamptz NOT NULL,
    end_at                   timestamptz CHECK (end_at IS NULL OR end_at > start_at),
    status                   text NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused', 'archived')),
    reminder_offsets_minutes integer[] NOT NULL DEFAULT '{1440,60}',
    notify_on_boundary       boolean NOT NULL DEFAULT true,
    created_by               uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at               timestamptz NOT NULL DEFAULT now(),
    updated_at               timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX schedules_org_idx ON schedules (organization_id) WHERE status <> 'archived';
CREATE INDEX schedules_team_idx ON schedules (team_id);

-- Rotation configuration is versioned. A version applies from effective_from until the next
-- version starts, so editing a schedule never rewrites assignments that already happened.
CREATE TABLE schedule_versions (
    id                 uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id        uuid NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    effective_from     timestamptz NOT NULL,
    rotation_interval  integer NOT NULL CHECK (rotation_interval BETWEEN 1 AND 8760),
    rotation_unit      text NOT NULL CHECK (rotation_unit IN ('hours', 'days', 'weeks')),
    created_by         uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at         timestamptz NOT NULL DEFAULT now(),
    UNIQUE (schedule_id, effective_from)
);

CREATE TABLE schedule_rotation_participants (
    version_id  uuid NOT NULL REFERENCES schedule_versions(id) ON DELETE CASCADE,
    layer       text NOT NULL CHECK (layer IN ('primary', 'secondary')),
    position    integer NOT NULL CHECK (position >= 0),
    user_id     uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    PRIMARY KEY (version_id, layer, position)
);

CREATE TABLE schedule_overrides (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    schedule_id  uuid NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    layer        text NOT NULL CHECK (layer IN ('primary', 'secondary')),
    user_id      uuid NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    start_at     timestamptz NOT NULL,
    end_at       timestamptz NOT NULL CHECK (end_at > start_at),
    reason       text NOT NULL DEFAULT '',
    created_by   uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    canceled_at  timestamptz,
    canceled_by  uuid REFERENCES users(id) ON DELETE SET NULL
);
CREATE INDEX schedule_overrides_schedule_idx ON schedule_overrides (schedule_id, start_at);

CREATE TABLE integrations (
    id                    uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id       uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    team_id               uuid REFERENCES teams(id) ON DELETE CASCADE,
    kind                  text NOT NULL CHECK (kind IN ('discord')),
    name                  text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    enabled               boolean NOT NULL DEFAULT true,
    events                text[] NOT NULL,
    mention_users         boolean NOT NULL DEFAULT false,
    message_prefix        text NOT NULL DEFAULT '',
    max_attempts          integer NOT NULL DEFAULT 6 CHECK (max_attempts BETWEEN 1 AND 20),
    secret_ciphertext     bytea NOT NULL,
    secret_nonce          bytea,
    secret_hint           text NOT NULL,
    last_delivery_status  text,
    last_delivery_at      timestamptz,
    created_by            uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at            timestamptz NOT NULL DEFAULT now(),
    updated_at            timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX integrations_org_idx ON integrations (organization_id);

-- Persistent event queue. dedupe_key is derived deterministically from the schedule transition
-- so the planner can run any number of times (and on any number of workers) without duplicates.
CREATE TABLE notification_events (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id  uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    schedule_id      uuid REFERENCES schedules(id) ON DELETE CASCADE,
    team_id          uuid REFERENCES teams(id) ON DELETE SET NULL,
    event_type       text NOT NULL,
    -- Integrations subscribe to tags; one event can carry several (rotation_changed + primary_changed).
    tags             text[] NOT NULL DEFAULT '{}',
    dedupe_key       text NOT NULL UNIQUE,
    payload          jsonb NOT NULL,
    scheduled_for    timestamptz NOT NULL,
    status           text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'dispatched', 'superseded', 'skipped')),
    locked_until     timestamptz,
    exclude_integration_id uuid,
    created_at       timestamptz NOT NULL DEFAULT now(),
    dispatched_at    timestamptz
);
CREATE INDEX notification_events_due_idx ON notification_events (scheduled_for) WHERE status = 'pending';
CREATE INDEX notification_events_schedule_idx ON notification_events (schedule_id, status);

-- One delivery per (event, integration). Workers claim rows with FOR UPDATE SKIP LOCKED and a
-- lease (locked_until); a crashed worker's lease simply expires and another worker retries.
CREATE TABLE notification_deliveries (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    event_id         uuid NOT NULL REFERENCES notification_events(id) ON DELETE CASCADE,
    integration_id   uuid NOT NULL REFERENCES integrations(id) ON DELETE CASCADE,
    organization_id  uuid NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    status           text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'sending', 'succeeded', 'failed', 'canceled')),
    attempts         integer NOT NULL DEFAULT 0,
    next_attempt_at  timestamptz NOT NULL DEFAULT now(),
    locked_until     timestamptz,
    last_status_code integer,
    last_error       text,
    delivered_at     timestamptz,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    UNIQUE (event_id, integration_id)
);
CREATE INDEX notification_deliveries_due_idx ON notification_deliveries (next_attempt_at) WHERE status IN ('pending', 'sending');
CREATE INDEX notification_deliveries_org_idx ON notification_deliveries (organization_id, created_at DESC);

CREATE TABLE notification_delivery_attempts (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    delivery_id  uuid NOT NULL REFERENCES notification_deliveries(id) ON DELETE CASCADE,
    attempt      integer NOT NULL,
    started_at   timestamptz NOT NULL,
    finished_at  timestamptz NOT NULL,
    status_code  integer,
    success      boolean NOT NULL,
    error        text
);
CREATE INDEX notification_delivery_attempts_delivery_idx ON notification_delivery_attempts (delivery_id);

CREATE TABLE audit_logs (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id  uuid REFERENCES organizations(id) ON DELETE CASCADE,
    actor_user_id    uuid REFERENCES users(id) ON DELETE SET NULL,
    action           text NOT NULL,
    target_type      text NOT NULL,
    target_id        uuid,
    metadata         jsonb NOT NULL DEFAULT '{}',
    created_at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX audit_logs_org_idx ON audit_logs (organization_id, created_at DESC);
