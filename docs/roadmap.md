# Roadmap

## 0.1 (this release)

- Google OIDC sign-in, sessions, CSRF, registration policies, admin bootstrap
- Organizations, teams, roles, invitations, audit log
- Rotation engine with versions, overrides, DST-correct local-time hand-overs
- Discord integration with persistent, retried, deduplicated delivery and history
- Dashboard, schedule timeline, calendar and list views, rotation preview
- Docker Compose deployment, CI, metrics, health probes
- iCalendar subscription links per person and per schedule
- Shift swaps requested by participants and accepted by the colleague
- Helm chart and multi-arch images on Docker Hub

## Next

- End-to-end browser tests (Playwright) for the primary workflow
- Weekday and time-of-day restrictions on rotation layers
- Incoming alerts: Grafana and Prometheus Alertmanager webhooks routed to whoever is on call
- Generic outgoing webhooks, Slack and email
- GitHub and generic OIDC sign-in

## Later

- Escalation policies with acknowledgement
- Phone and SMS through pluggable providers
- Terraform provider
