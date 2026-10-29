# Architecture

OncallKit is a modular monolith: one Rust binary (`oncallkit`) that serves the HTTP API and runs the background worker, a React single-page app, and PostgreSQL as the only stateful dependency.

```
crates/api/src
├── main.rs             CLI: serve | worker | migrate | bootstrap-admin | seed-demo
├── config.rs           Environment configuration and production safety checks
├── routes.rs           Router and middleware (request ids, tracing, limits, timeouts, headers)
├── auth/               Google OIDC, sessions, CSRF, request extractors (CurrentUser, OrgContext)
├── rbac.rs             Roles and permissions
├── organizations.rs    Organizations, memberships, invitations
├── teams.rs
├── schedules/
│   ├── engine.rs       The rotation engine (pure, no I/O)
│   ├── repo.rs         Versioned storage ↔ engine spec
│   └── mod.rs          HTTP handlers
├── notifications/
│   ├── planner.rs      Schedule transitions → persisted events
│   ├── dispatcher.rs   Events → deliveries → Discord, with leases and retries
│   ├── message.rs      Discord message rendering
│   └── routes.rs       Delivery history and health
├── integrations/       Discord integration settings and webhook client
├── audit.rs, observability.rs, crypto.rs, worker.rs
migrations/             SQLx migrations (embedded in the binary)
apps/web/               React + TypeScript + Vite + Tailwind
```

## The rotation engine

`schedules/engine.rs` answers "who is on call at T" from configuration alone:

- A schedule has a start, an optional end and an IANA timezone.
- Rotation configuration is a list of **versions**, each with an `effective_from`, an interval and unit, and ordered primary and secondary participant lists. A version applies until the next one begins. Every version counts shifts from the schedule's start, so the order continues predictably after an edit.
- Shift *n* starts at `start + n × interval`. For `hours` this is absolute time. For `days` and `weeks` it is calendar arithmetic in the schedule's timezone, so a 09:00 hand-over stays at 09:00 through daylight-saving changes (the shift that spans the change is 23 or 25 hours long). A wall-clock time that does not exist (inside a DST gap) resolves to the first valid minute after the gap; an ambiguous one resolves to its earlier occurrence.
- The participant for shift *n* is `participants[n mod len]`.
- **Overrides** replace the rotation assignee for their window, clipped to the schedule's active range. Where overrides overlap, the most recently created wins.
- All intervals are half-open: a shift includes its start and excludes its end.

Because nothing is stored per shift, there is no job that "moves" the on-call person. Restarts, missed timers and clock skew between workers cannot make the answer wrong.

### Edits and history

- Before a schedule starts, editing the rotation replaces its configuration.
- After it starts, editing the rotation inserts a new version effective **now** or at the **next hand-over** (the editor chooses). Earlier versions are untouched, so past assignments never change. Versions scheduled after the new one are replaced.
- The start and timezone are frozen once a schedule has started.
- Removing a member from the organization starts a new version without them (now), edits not-yet-started versions in place, and cancels their future overrides.
- Archiving a schedule stops coverage and withdraws pending notifications but keeps all rows.

## Notifications

### Events

`notification_events` is the persistent queue of things worth announcing. Each row has a `dedupe_key` with a unique constraint:

| Event | Key |
| --- | --- |
| Hand-over | `handover:{schedule}:{unix_ts}:{primary}:{secondary}` |
| Reminder | `reminder:{schedule}:{layer}:{shift_start}:{user}:{offset_minutes}` |
| Schedule created | `schedule_created:{schedule}` |
| Override created / canceled | `override_created:{override}` / `override_canceled:{override}` |
| Delivery failed | `delivery_failed:{delivery}` |

Events carry tags (a hand-over can be `rotation_changed` + `primary_changed`), and integrations subscribe to tags. One hand-over is one message per channel, even when both layers change at once.

### The planner

Every minute, and immediately after any schedule change, the planner computes for each schedule:

- hand-overs in `[max(now − 6 h, schedule.created_at), now + longest reminder + 2 h)`;
- reminders for upcoming shifts whose send time is not more than 10 minutes in the past.

It inserts missing events (`ON CONFLICT DO NOTHING`, reviving a superseded one if it becomes relevant again) and marks pending planned events that are no longer in the computed set as `superseded`. Planning for a schedule is serialised with a transaction-scoped advisory lock.

**Edits after a reminder was sent:** a sent reminder stays sent. If the edit changes who takes the next shift, the new person gets their own reminder (a different key) as long as its send time has not passed by more than the grace period, and the channel also receives a "schedule updated" message. Pending reminders for the old assignee are superseded and never sent.

**Worker downtime:** events are written ahead of time, so a worker that is down at the hand-over sends it when it comes back. Hand-overs up to six hours old are still planned on restart; reminders for shifts that already started are skipped as stale.

### Dispatch and delivery

1. The dispatcher claims due events with `UPDATE … WHERE id IN (SELECT … FOR UPDATE SKIP LOCKED)` and a lease, inserts one `notification_deliveries` row per matching enabled integration (unique on `(event_id, integration_id)`), and marks the event `dispatched`.
2. Delivery workers claim deliveries the same way, set them to `sending` with a 60-second lease, and send them.
3. Outcomes: 2xx → `succeeded`. 429, 5xx, timeouts and connection errors → retried after `Retry-After` if Discord sent one, otherwise exponential backoff (30 s doubling to a 1 h cap, scaled by random jitter in [0.5, 1]). Other 4xx (deleted webhook, bad request) → `failed` immediately. After `max_attempts` → `failed`, and a `delivery_failed` event is raised for the organization's *other* channels.
4. Every attempt is recorded in `notification_delivery_attempts`.

**Delivery guarantee:** at-least-once. If a worker dies after Discord accepted a message but before recording it, the lease expires and the message can be sent a second time. Discord webhooks have no idempotency key, so exactly-once is not possible end to end; the lease is far longer than the 10-second request timeout to make this window small.

## Authentication and sessions

- `GET /api/v1/auth/google/start` creates a single-use `oauth_flows` row (hashed state, nonce, PKCE verifier, sanitized return path; 10 minutes) and a browser-binding cookie, then redirects to Google.
- The callback deletes the flow row, checks the binding cookie, exchanges the code with the PKCE verifier and verifies the ID token (signature against Google's keys, issuer, audience, expiry, nonce) using the `openidconnect` crate. The email must be verified by Google.
- The user is found by `(provider, subject)`. New users are created only if the registration policy allows it. Email is updated on each login but never used to link accounts.
- A fresh session is created at each sign-in. The browser holds a random token; the database holds its SHA-256 hash, the CSRF token, expiry and the active organization. Logout and "sign out other sessions" revoke rows server-side.
- Non-GET requests require `X-CSRF-Token` to match the session's token (constant-time comparison). Cookies are `SameSite=Lax`.

The provider code is isolated in `auth/google.rs` and produces a `VerifiedIdentity`; adding GitHub or a generic OIDC provider means adding another producer of that struct and a pair of routes. Authorization is unaffected.

## Multi-tenancy

Every organization-scoped handler takes an `OrgContext` (user, active organization, role). Queries always filter by `organization_id`; IDs from another organization behave as if they do not exist (404). Before people or teams are referenced (participants, overrides, team members, integration scope), the backend checks they belong to the same organization.

## Observability

- `GET /healthz`: process is up. `GET /readyz`: database reachable (2-second timeout).
- `GET /metrics` (optionally bearer-protected): `oncallkit_http_requests_total`, `oncallkit_http_request_duration_seconds`, `oncallkit_jobs_processed_total`, `oncallkit_notification_deliveries_total{outcome}`, `oncallkit_notification_retries_total`, `oncallkit_notification_deliveries_pending`.
- Every request gets an `x-request-id` (propagated to the response). Logs are JSON in production. Only paths are logged (query strings can hold OAuth codes); cookies, CSRF tokens and authorization headers are marked sensitive; webhook URLs are never logged.
