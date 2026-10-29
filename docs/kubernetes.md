# Kubernetes (Helm)

OncallKit ships a Helm chart in [`deploy/helm/oncallkit`](../deploy/helm/oncallkit). Released
versions are published to Docker Hub together with the images, so you can install straight from
the registry.

## What gets deployed

| Component | Kind | Notes |
| --- | --- | --- |
| `web` | Deployment + Service | nginx serving the app and proxying `/api` to the API, so the browser sees one origin. The Ingress points here. |
| `api` | Deployment + Service + PodDisruptionBudget | Applies migrations on start (under a database lock, so replicas take turns) and runs the background worker too. |
| `worker` | Deployment (optional) | Set `worker.enabled=true` to run the worker on its own; API pods then stop running it. |
| `postgresql` | StatefulSet (optional) | A single PostgreSQL 16 for trying OncallKit out or small teams. Use a managed database in production. |
| Secret | | Encryption key, Google credentials, database URL. Generated values are kept on upgrades. |

Pods run as non-root with a read-only root filesystem, no service account token and all
capabilities dropped.

## Install

You need: a cluster with an ingress controller, a hostname, a TLS certificate for it (for example
from cert-manager), PostgreSQL 14 or newer, and a Google OAuth client
([docs/google-oauth.md](google-oauth.md)) whose redirect URI is
`https://<host>/api/v1/auth/google/callback`.

```bash
helm install oncallkit oci://registry-1.docker.io/ilhamriky/oncallkit \
  --namespace oncallkit --create-namespace \
  --set publicUrl=https://oncall.example.com \
  --set database.url='postgres://oncallkit:PASSWORD@db.example.com:5432/oncallkit?sslmode=require' \
  --set google.clientId=CLIENT_ID --set google.clientSecret=CLIENT_SECRET \
  --set ingress.enabled=true --set ingress.className=nginx \
  --set ingress.tls.enabled=true \
  --set-string ingress.annotations."cert-manager\.io/cluster-issuer"=letsencrypt
```

Then make yourself the first administrator and sign in with that Google account:

```bash
kubectl -n oncallkit exec deploy/oncallkit-api -- oncallkit bootstrap-admin --email you@example.com
```

Pin a version with `--version 0.2.0`. Without it Helm installs the newest chart, whose images
match its `appVersion`.

### Keep secrets out of values

Instead of passing credentials with `--set`, create a Secret yourself and point the chart at it:

```bash
kubectl -n oncallkit create secret generic oncallkit \
  --from-literal=DATABASE_URL='postgres://…' \
  --from-literal=ENCRYPTION_KEY="$(openssl rand -base64 32)" \
  --from-literal=GOOGLE_CLIENT_ID=… --from-literal=GOOGLE_CLIENT_SECRET=…
helm install oncallkit oci://registry-1.docker.io/ilhamriky/oncallkit -n oncallkit \
  --set publicUrl=https://oncall.example.com --set existingSecret=oncallkit \
  --set ingress.enabled=true --set ingress.className=nginx
```

The keys it may hold: `DATABASE_URL`, `ENCRYPTION_KEY`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`,
`METRICS_TOKEN`. This works with External Secrets, Sealed Secrets or Vault as well.

### The encryption key

`ENCRYPTION_KEY` encrypts Discord webhook URLs in the database. If you do not provide one, the
chart generates it on install, keeps it on every upgrade and keeps the Secret when the release is
uninstalled. **Back it up**: restoring the database without it means re-entering every webhook.

```bash
kubectl -n oncallkit get secret oncallkit -o jsonpath='{.data.ENCRYPTION_KEY}' | base64 -d
```

### Trying it out without a database

`postgresql.enabled=true` runs a single PostgreSQL inside the release with a generated password
and an 8 Gi volume. Back up that volume, or move to a managed database before relying on it.

## Common settings

| Value | Default | Meaning |
| --- | --- | --- |
| `publicUrl` | (required) | The address people open. https in production. |
| `appEnv` | `production` | `development` allows http and development sign-in. |
| `auth.registrationMode` | invite only in production | `open`, `invite_only` or `domains` with `auth.allowedEmailDomains`. |
| `auth.orgCreation` | instance admins in production | `anyone` or `instance_admins`. |
| `api.replicaCount` / `web.replicaCount` | 2 / 2 | Any number; workers never double-send. |
| `worker.enabled` | false | Run the worker as its own Deployment. |
| `database.maxConnections` | 10 | Pool size per pod. Mind your database's connection limit. |
| `metricsToken` | | Protects `/metrics` (served by the API on port 8080). |
| `image.tag` | chart appVersion | Image version. |

Everything else is in [`values.yaml`](../deploy/helm/oncallkit/values.yaml).

## Upgrades

```bash
helm upgrade oncallkit oci://registry-1.docker.io/ilhamriky/oncallkit -n oncallkit --reuse-values --version 0.3.0
```

New API pods apply migrations when they start. Migrations are additive, so old pods keep working
during the rollout. Back up the database before upgrading across minor versions.

## Publishing (maintainers)

Pushing a tag `vX.Y.Z` runs [`.github/workflows/release.yml`](../.github/workflows/release.yml),
which builds `oncallkit-api` and `oncallkit-web` natively for amd64 and arm64, publishes them as
`X.Y.Z`, `X.Y` and `latest`, and pushes the chart as `oci://registry-1.docker.io/<namespace>/oncallkit`.
It needs the repository secrets `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` (a Docker Hub access token
with read and write access), and optionally the variable `DOCKERHUB_NAMESPACE` when publishing to a
Docker Hub organization other than `ilhamriky`.
