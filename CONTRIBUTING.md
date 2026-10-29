# Contributing to OncallKit

Thank you for helping. Bug reports, documentation fixes and code are all welcome.

## Before you start

- For anything bigger than a small fix, open an issue first so we can agree on the approach.
- Security problems go through [SECURITY.md](SECURITY.md), not public issues.
- By contributing you agree that your contribution is licensed under the Apache License 2.0.

## Development setup

See [Development in the README](README.md#development). You need Rust (stable), Node 22+, and PostgreSQL 14+ (or Docker).

## Making changes

- **Schedule logic belongs in `schedules/engine.rs`** and must stay free of I/O. Add a unit test for every behaviour change, especially around boundaries and timezones.
- **Schema changes** go in a new file in `migrations/` (`YYYYMMDDHHMMSS_description.sql`). Never edit a migration that has been released.
- **Every protected endpoint** takes `OrgContext` (or `CurrentUser`), checks a permission, and filters by `organization_id`. Add an integration test for the authorization rule.
- **Never log secrets.** Webhook URLs, tokens and cookies must not appear in logs, errors or API responses.
- **Frontend:** follow the existing design rules: one typeface, tone steps rather than lines and shadows, no gradients, sentence case, and real data only (no placeholder numbers).

## Checks

Run before pushing (CI runs the same):

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
DATABASE_URL=postgres://postgres:postgres@localhost:5432/oncallkit_test cargo test --workspace
cd apps/web && npm ci && npm run lint && npm run typecheck && npm test && npm run build
```

## Commits and pull requests

- Write commit messages in the imperative ("Add weekday restrictions to rotations").
- Keep pull requests focused, describe what changed and how you tested it, and update docs when behaviour changes.

## Releases

We use [Semantic Versioning](https://semver.org/). Releases are tagged `vX.Y.Z` and their notes list breaking changes and migrations first. Dependencies are kept current through Dependabot; security updates are merged promptly.
