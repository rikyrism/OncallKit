# Deploying OncallKit

## Production checklist

- [ ] `APP_ENV=production`
- [ ] `PUBLIC_URL` is the https URL people use (the server refuses http in production)
- [ ] TLS terminates in front of the `web` container (Caddy, Traefik, nginx, a cloud load balancer)
- [ ] `ENCRYPTION_KEY` set (`openssl rand -base64 32`) and stored in your secret manager; **back it up**, because losing it makes stored webhook URLs unreadable
- [ ] Google OAuth client with redirect URI `<PUBLIC_URL>/api/v1/auth/google/callback` ([guide](google-oauth.md))
- [ ] `REGISTRATION_MODE=invite_only` (the production default) or `domains`
- [ ] `ORG_CREATION=instance_admins` (the production default), and the first administrator bootstrapped
- [ ] `AUTH_DEV_LOGIN` unset (the server refuses to start with it in production)
- [ ] `METRICS_TOKEN` set if `/metrics` is reachable from outside your network
- [ ] `TRUST_PROXY_HEADERS=true` only when a proxy you control sets `X-Forwarded-For`
- [ ] Database backups scheduled and a restore tested

## Docker Compose

```sh
cp .env.example .env    # then edit
docker compose up -d --build
docker compose exec api oncallkit bootstrap-admin --email you@example.com
```

`docker-compose.yml` runs PostgreSQL, the API (with the worker built in) and nginx serving the web app on port 8080 and proxying `/api`. Put your TLS proxy in front of port 8080.

Migrations run automatically when `oncallkit serve` or `oncallkit worker` starts. To run them separately (for example in a release step): `oncallkit migrate`.

## Scaling and workers

Every instance of `oncallkit serve` runs a worker unless `RUN_WORKER=false`. Running several is safe: work is claimed with `FOR UPDATE SKIP LOCKED` and leases, planning is serialised per schedule with advisory locks, and every event has a unique deduplication key. A common layout is:

- N API replicas with `RUN_WORKER=false`
- 1 or 2 `oncallkit worker` processes

The rate limiter for sign-in endpoints is per process; with several API replicas, also rate-limit `/api/v1/auth/` at your proxy.

On Kubernetes, use `/healthz` as the liveness probe and `/readyz` as the readiness probe. The container runs as UID 10001 and needs no writable filesystem.

## Backups and restore

All state is in PostgreSQL. Back it up with your usual tooling, for example:

```sh
docker compose exec -T db pg_dump -U oncallkit -Fc oncallkit > oncallkit-$(date +%F).dump
```

Restore into an empty database:

```sh
docker compose stop api
docker compose exec -T db dropdb -U oncallkit oncallkit
docker compose exec -T db createdb -U oncallkit oncallkit
docker compose exec -T db pg_restore -U oncallkit -d oncallkit < oncallkit-2026-10-29.dump
docker compose start api
```

The `ENCRYPTION_KEY` is not in the database. Keep it with the backups (separately and securely), or restored integrations will need their webhook URLs entered again.

After a restore, pending notifications resume where they were. The planner re-creates any hand-overs from the last six hours that are missing, and deduplication keys prevent repeats of anything already recorded as sent.

## Upgrades

1. Read the release notes for breaking changes.
2. Back up the database.
3. Pull or build the new images and restart: `docker compose up -d --build`.
4. Migrations apply on start. They are forward-only; to roll back, restore the backup taken in step 2 and run the previous version.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Server exits at start with a configuration error | The message names the variable. Production refuses http URLs, a missing `ENCRYPTION_KEY` and development sign-in. |
| "Google OpenID Connect discovery failed" | The container needs outbound https to `accounts.google.com`. |
| Sign-in returns to the login page with "expired or opened in another browser" | `PUBLIC_URL` must exactly match the address in the browser, and the redirect URI registered at Google. Cookies are scoped to it. |
| "Registration is closed for this address" | The server is invite-only. Invite the address, or use `bootstrap-admin`. |
| Discord test fails with 401 or 404 | The webhook was deleted or the URL is wrong. Create a new one and paste it into the integration. |
| Deliveries stay "Waiting" | Is a worker running (`RUN_WORKER` or `oncallkit worker`)? Check its logs and `oncallkit_notification_deliveries_pending`. |
| "could not decrypt a stored secret" | `ENCRYPTION_KEY` changed. Restore the old key, or re-enter webhook URLs. |
| Times look shifted | Times display in the schedule's timezone by default; the schedule page can switch to your browser's timezone. |
