# Known limitations

OncallKit 0.1 is early software. These are the gaps we know about.

## Security

- **No external security review yet.** The authentication, session, CSRF, tenant isolation and SSRF paths have automated tests, but nobody independent has audited them.
- **The live Google exchange is not covered by automated tests.** The flow-state handling around it is, and ID-token verification (signature, issuer, audience, expiry, nonce) is done by the `openidconnect` crate. Test sign-in with a real client before going live.
- **Sign-in rate limiting is per process.** With several API replicas, add rate limiting at the proxy.
- **Development sign-in** (`AUTH_DEV_LOGIN`) lets anyone sign in as anyone. It is refused in production, but do not expose a development server.
- Without `ENCRYPTION_KEY` (allowed only outside production), webhook URLs are stored in plain text and the integrations page says so.
- Invitation emails are not sent; administrators copy the one-time link and share it themselves.

## Notifications

- **At-least-once delivery.** If a worker crashes after Discord accepted a message but before recording it, the message can be sent twice. Discord webhooks offer no idempotency key.
- Hand-overs missed during more than six hours of worker downtime are not announced afterwards. Reminders whose shift has started are skipped.
- Discord is the only channel so far. Slack, email, Grafana and Alertmanager integrations are on the [roadmap](roadmap.md).

## Scheduling

- One rotation cadence per schedule, shared by its primary and secondary layers. Their participant lists are independent.
- No per-day restrictions (for example weekdays only, or business hours) yet.
- A schedule's start and timezone cannot change after it starts; create a new schedule instead.
- The planner reads all non-archived schedules every minute. That is fine for hundreds of schedules; thousands would want incremental planning.

## Calendar feeds and swaps

- A calendar link works for anyone who has it, like a password. Revoke it from the profile page if it leaks.
- Calendar apps decide how often they re-read a feed. Apple Calendar and Outlook follow the hourly hint; Google Calendar can take up to a day, so recent swaps and overrides may show late there. OncallKit itself is always current.
- A new swap request is shown in the app (dashboard and Shift swaps page) but not announced in Discord. Accepted swaps are announced like any override.
- The swap form offers whole shifts. The API accepts any window the requester holds completely.

## Kubernetes

- The bundled PostgreSQL (`postgresql.enabled`) is a single instance without backups or failover. Use a managed database for anything you rely on.
- Rate limiting is per pod, as noted above; put limits on the ingress for the sign-in and calendar endpoints if the instance is public.

## Other

- No GitHub or generic OIDC sign-in yet (the code is structured for it).
- No email or name editing for other users; profiles come from Google.
- No end-to-end browser tests yet; the primary workflow is covered by API-level integration tests and component tests.
