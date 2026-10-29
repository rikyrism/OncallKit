# Security policy

## Reporting a vulnerability

Please do not open public issues for security problems.

Report vulnerabilities through **GitHub's private vulnerability reporting** (the "Report a vulnerability" button on the repository's Security tab). Include what you found, how to reproduce it, and the impact you expect.

We aim to acknowledge reports within 3 working days and to agree on a disclosure date with you, normally within 90 days. We credit reporters who want to be credited.

## Supported versions

Until 1.0, only the latest release receives security fixes.

## Scope

In scope: the OncallKit server, worker, web app, container images and deployment examples in this repository.

Out of scope: vulnerabilities that need a compromised host or database, development-only settings (`AUTH_DEV_LOGIN`, `DISCORD_API_BASE_OVERRIDE`) and denial of service through volume alone.

## Hardening notes for operators

See [docs/deployment.md](docs/deployment.md#production-checklist) and [docs/known-limitations.md](docs/known-limitations.md).
