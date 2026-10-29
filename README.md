<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/wordmark-dark.svg">
    <img src="docs/assets/wordmark-light.svg" alt="OncallKit" width="360">
  </picture>
</p>

<p align="center">
  <strong>Your on-call, under control.</strong><br>
  Self-hosted on-call scheduling: rotations that hand over on time in every timezone, and Discord told the moment someone new holds the pager.
</p>

<p align="center">
  <a href="https://github.com/rikyrism/OncallKit/actions/workflows/ci.yml"><img src="https://github.com/rikyrism/OncallKit/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-f2c200" alt="Apache-2.0 License"></a>
  <img src="https://img.shields.io/badge/backend-Rust%20%C2%B7%20Axum-111111" alt="Rust and Axum">
  <img src="https://img.shields.io/badge/frontend-React%20%C2%B7%20TypeScript-111111" alt="React and TypeScript">
  <img src="https://img.shields.io/badge/database-PostgreSQL-111111" alt="PostgreSQL">
</p>

<p align="center">
  <a href="#quick-start">Quick start</a> ·
  <a href="#show-it-from-your-laptop-for-free">Share for free</a> ·
  <a href="#deploy">Deploy</a> ·
  <a href="#api">API</a> ·
  <a href="#how-it-works">How it works</a>
</p>

---

Put your engineers in order, choose how often the pager moves, and OncallKit works out every shift from there: who is on call right now, who is next, and when. Hand-overs and reminders land in your team's Discord channel. One Rust binary, one PostgreSQL database, nothing else to run.

> **Status: early (0.1).** The core is implemented and tested, but it has not had an external security review or long production use. See [Limitations](#limitations).

## Features

**Schedules that are always right**
- Rotate every N hours, days or weeks, with a start date and either an end date or none.
- Separate primary and secondary rotations, each with its own order.
- Any IANA timezone. A 09:00 WIB hand-over stays at 09:00 through daylight-saving changes elsewhere.
- Overrides for leave, holidays and swaps; the rotation resumes by itself afterwards.
- Shift swaps between colleagues: ask someone to cover a shift (or trade one of yours for one of theirs); when they accept, the overrides are made for you.
- "Who is on call at T?" is calculated from the schedule, never stored in a field a cron job updates, so it is correct after restarts and missed timers.
- Edits start a new version from now (or from the next hand-over). Past shifts never change.
- A live preview while you build a schedule, computed by the same engine that runs in production.
- Subscribe from Google Calendar, Apple Calendar or Outlook: a private iCal link for your own shifts or a whole schedule, kept up to date automatically.

**Discord that never lies**
- Hand-over announcements, reminders before a shift (e.g. 24 h and 1 h), schedule changes and overrides, each switchable per channel.
- Optional @mentions of the person going on call, restricted to them: no `@everyone` accidents.
- Every message is stored before sending, retried with exponential backoff and jitter, respects Discord's `Retry-After`, and is deduplicated even with several workers.
- Delivery history shows every attempt and exactly what Discord answered. **Send test** reports a failure as a failure.

**Teams and access**
- Google sign-in (OpenID Connect with PKCE, state and nonce) keyed on Google's stable account id, not on email.
- Open, invite-only or allowed-domain registration, plus a CLI bootstrap for the first administrator.
- Organizations, teams and four roles (owner, admin, scheduler, member), enforced by the server on every request.
- Expiring, single-use invitation links tied to the invited address. An audit log of every change.

**Built to be hosted**
- Docker Compose for everything, health and readiness probes, Prometheus metrics, JSON logs, strict CSP.
- Webhook URLs encrypted at rest (AES-256-GCM), never returned by the API and never logged.
- Light and dark themes, usable on desktop, tablet and phone.

## Quick start

Run everything with Docker:

```bash
git clone https://github.com/rikyrism/OncallKit.git
cd OncallKit
cp .env.example .env    # set POSTGRES_PASSWORD and ENCRYPTION_KEY (openssl rand -base64 32)
docker compose up --build
```

Open <http://localhost:8080>.

Without Google credentials yet? Set `AUTH_DEV_LOGIN=true` in `.env` to sign in with any email address (local development only; the server refuses it in production). Load sample people and schedules with:

```bash
docker compose exec api oncallkit seed-demo --owner-email you@example.com
```

### Development

Requirements: Rust (stable), Node.js 22+, Docker.

```bash
# 1. Database
docker compose up -d db

# 2. Backend on :8080 (runs migrations on start, reads .env)
DATABASE_URL=postgres://oncallkit:<password>@localhost:5432/oncallkit cargo run -p oncallkit-api -- serve

# 3. Frontend on :5173 (proxies /api to :8080)
cd apps/web
npm ci
npm run dev
```

No local Rust? `scripts/cargo-docker.sh <cargo args>` runs cargo inside the official Rust image.

| Command | Where | What it does |
| --- | --- | --- |
| `cargo test` | root | Engine unit tests, API tests against PostgreSQL, notification tests against a mock Discord (needs `DATABASE_URL`) |
| `cargo clippy --all-targets -- -D warnings` | root | Lints |
| `npm test` | `apps/web/` | Vitest + Testing Library |
| `npm run lint` · `npm run typecheck` | `apps/web/` | ESLint and TypeScript |

## Show it from your laptop for free

No server and no domain needed. A free [Cloudflare quick tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/do-more-with-tunnels/trycloudflare/) gives your laptop a public HTTPS address:

```bash
brew install cloudflared            # once
./scripts/tunnel.sh --dev-login     # Ctrl+C to stop
```

The script opens the tunnel, restarts the Docker stack with `PUBLIC_URL` set to the tunnel address (so cookies, sign-in and links in Discord messages use it), and prints something like `https://something.trycloudflare.com`. Open OncallKit at that address, not at localhost. Ctrl+C closes the tunnel and switches the stack back to <http://localhost:8080>. Your computer has to stay on, and the address changes on every run.

**Choose how people sign in:**

- `--dev-login` signs in with any email address, without Google. **Anyone who has the address can sign in as anyone**, so share it only with people you trust and close the tunnel when you are done.
- Without the flag, Google sign-in is used (`GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` in `.env`). Because the address is new every run, add the printed redirect URI (`https://….trycloudflare.com/api/v1/auth/google/callback`) to your Google OAuth client each time. For a fixed address, use a [named tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/get-started/) with your own domain.

## Deploy

`docker-compose.yml` is the production layout too: PostgreSQL (not exposed), the API with the worker built in, and nginx serving the web app and proxying `/api`. Put a TLS proxy (Caddy, Traefik, a cloud load balancer) in front of port 8080.

```bash
cp .env.example .env    # APP_ENV=production, https PUBLIC_URL, ENCRYPTION_KEY, Google client
docker compose up -d --build
docker compose exec api oncallkit bootstrap-admin --email you@example.com
```

### Kubernetes

A Helm chart is published to Docker Hub with every release, next to multi-arch (amd64, arm64) images `ilhamriky/oncallkit-api` and `ilhamriky/oncallkit-web`:

```bash
helm install oncallkit oci://registry-1.docker.io/ilhamriky/oncallkit -n oncallkit --create-namespace \
  --set publicUrl=https://oncall.example.com \
  --set database.url='postgres://…' \
  --set google.clientId=… --set google.clientSecret=… \
  --set ingress.enabled=true --set ingress.className=nginx
```

Add `--set postgresql.enabled=true` instead of `database.url` to try it with a bundled database. See [docs/kubernetes.md](docs/kubernetes.md) for secrets, TLS, scaling and upgrades.

In production the server refuses an http `PUBLIC_URL`, a missing encryption key and development sign-in, and defaults to invite-only registration. A small VM (**1 vCPU, 1 GB RAM**) should be enough to start (it has not been load-tested yet). Workers can run on any number of instances; they never double-send. See [docs/deployment.md](docs/deployment.md) for the checklist, scaling, backups, upgrades and troubleshooting, and [docs/google-oauth.md](docs/google-oauth.md) and [docs/discord.md](docs/discord.md) for setting up the integrations.

## API

Everything the web app does goes through a versioned REST API under `/api/v1`. For example, who is on call next Tuesday:

```bash
curl -b cookies.txt "https://oncall.example.com/api/v1/schedules/$SCHEDULE_ID/on-call-now?at=2026-11-10T03:00:00Z"
```

```json
{
  "at": "2026-11-10T03:00:00Z",
  "primary":   { "layer": "primary",   "user_id": "6f1c…", "start": "2026-11-10T02:00:00Z", "end": "2026-11-17T02:00:00Z", "source": { "kind": "rotation", "index": 1 } },
  "secondary": { "layer": "secondary", "user_id": "a93e…", "start": "2026-11-10T02:00:00Z", "end": "2026-11-17T02:00:00Z", "source": { "kind": "rotation", "index": 1 } },
  "next_rotation_at": "2026-11-17T02:00:00Z"
}
```

Also available: schedules and their previews, shift swap requests, iCalendar subscription links (`/calendar.ics`), upcoming hand-overs, assignments over a range, overrides, teams, members, invitations, integrations, delivery history and the audit log. Requests other than GET carry the session's CSRF token in `X-CSRF-Token`. The full contract is in [`docs/openapi.yaml`](docs/openapi.yaml), served at `/api/v1/openapi.yaml`.

## How it works

```
Browser (React)                    nginx               oncallkit (one Rust binary)
 ├─ Dashboard, schedules  ── HTTPS ──▶ /api ──────────▶ ├─ REST API (Axum): auth, RBAC, schedules, teams
 └─ Session cookie + CSRF                               ├─ Rotation engine (pure, fully unit tested)
                                                        ├─ Worker: plan → dispatch → deliver ──▶ Discord
                                                        └─ PostgreSQL: everything, including the job queue
```

- **The schedule is the source of truth.** Shift *n* starts at `start + n × interval` (in local calendar time for days and weeks), and the person is `participants[n mod len]`, with overrides laid on top. Nothing is stored per shift, so nothing can drift.
- **Edits are versions.** A change starts a new rotation version at a chosen moment; earlier versions keep answering for the past.
- **Notifications are planned ahead.** The planner writes upcoming hand-overs and reminders to the database with deterministic keys like `handover:{schedule}:{time}:{who}`, so planning twice, or on two workers, never duplicates, and an edit withdraws reminders that no longer apply.
- **Delivery uses leases.** Workers claim rows with `FOR UPDATE SKIP LOCKED` and a 60-second lease; a crashed worker's work is picked up by the next one. Each attempt and Discord's answer are recorded.

Deeper notes: [architecture](docs/architecture.md), [deployment](docs/deployment.md), [Kubernetes](docs/kubernetes.md), [Discord](docs/discord.md), [Google sign-in](docs/google-oauth.md), [known limitations](docs/known-limitations.md), [roadmap](docs/roadmap.md).

## Project structure

```
crates/api/            Rust backend (Axum, Tokio, SQLx)
  src/schedules/       Rotation engine (pure, no I/O), versioned storage, handlers
  src/notifications/   Planner, dispatcher, Discord message rendering, delivery history
  src/auth/            Google OIDC, sessions, CSRF, request extractors
  src/integrations/    Discord settings and the webhook client (SSRF-safe)
  tests/               API and notification tests against PostgreSQL and a mock Discord
migrations/            PostgreSQL migrations (embedded in the binary)
apps/web/              React + TypeScript + Vite + Tailwind
  src/features/        dashboard, schedules, teams, users, integrations, notifications, audit, settings
deploy/docker/         Dockerfiles and the nginx config template
deploy/helm/oncallkit/ Helm chart
scripts/               tunnel.sh for sharing from a laptop, cargo-docker.sh
docs/                  Architecture, deployment, OpenAPI, integrations, limitations, roadmap
```

## Limitations

- Notification delivery is at-least-once: if a worker dies right after Discord accepts a message, it can be sent twice (Discord webhooks have no idempotency key).
- Discord is the only channel so far; Slack, email, Grafana and Alertmanager are on the [roadmap](docs/roadmap.md).
- One rotation cadence per schedule (primary and secondary have separate orders but the same rhythm); no weekday or business-hours restrictions yet.
- Invitation emails are not sent; admins copy the one-time link.
- The live Google exchange is not covered by automated tests, and there are no browser end-to-end tests yet.

Full list: [docs/known-limitations.md](docs/known-limitations.md).

## Contributing

Issues and pull requests are welcome. Please run `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test`, `npm run lint`, `npm run typecheck` and `npm test` before opening a PR; CI runs the same checks. See [CONTRIBUTING.md](CONTRIBUTING.md) and the [Code of Conduct](CODE_OF_CONDUCT.md). Report security issues privately: [SECURITY.md](SECURITY.md).

## License

[Apache-2.0](LICENSE)

OncallKit is an independent project and is not affiliated with Grafana Labs, Google or Discord.
