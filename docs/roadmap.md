# Roadmap

## 0.1 (this release)

- Google OIDC sign-in, sessions, CSRF, registration policies, admin bootstrap
- Organizations, teams, roles, invitations, audit log
- Rotation engine with versions, overrides, DST-correct local-time hand-overs
- Discord integration with persistent, retried, deduplicated delivery and history
- Dashboard, schedule timeline, calendar and list views, rotation preview
- Docker Compose deployment, CI, metrics, health probes

## Next

- End-to-end browser tests (Playwright) for the primary workflow
- Shift swaps requested by participants and approved by schedulers
- Weekday and time-of-day restrictions on rotation layers
- iCal feed per person and per schedule
- Incoming alerts: Grafana and Prometheus Alertmanager webhooks routed to whoever is on call
- Generic outgoing webhooks, Slack and email
- GitHub and generic OIDC sign-in
- Helm chart

## Later

- Escalation policies with acknowledgement
- Phone and SMS through pluggable providers
- Terraform provider
