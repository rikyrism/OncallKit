#!/usr/bin/env bash
# Show OncallKit running on this computer through a free Cloudflare quick tunnel.
#
#   ./scripts/tunnel.sh              # sign in with Google (needs GOOGLE_CLIENT_ID/SECRET in .env)
#   ./scripts/tunnel.sh --dev-login  # sign in with any email, no Google (see the warning below)
#   Ctrl+C                           # close the tunnel and switch back to http://localhost
#
# It opens the tunnel first, then restarts the Docker Compose stack with PUBLIC_URL set to the
# https://….trycloudflare.com address, so cookies, sign-in redirects and links in Discord
# messages all use it. The address changes every run, and this computer must stay on.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${WEB_PORT:-8080}"
STATE="$ROOT/.tunnel"
OVERRIDE="$STATE/compose.tunnel.yml"
COMPOSE=(docker compose --project-directory "$ROOT" -f "$ROOT/docker-compose.yml")
DEV_LOGIN=false

for arg in "$@"; do
  case "$arg" in
    --dev-login) DEV_LOGIN=true ;;
    -h | --help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown option: $arg (try --help)"; exit 1 ;;
  esac
done

export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
for cmd in cloudflared docker curl; do
  command -v "$cmd" >/dev/null || { echo "Missing '$cmd'. Install it first: brew install cloudflared (and Docker Desktop or OrbStack)."; exit 1; }
done
docker info >/dev/null 2>&1 || { echo "Docker is not running. Start Docker Desktop or OrbStack first."; exit 1; }

if [ ! -f "$ROOT/.env" ]; then
  echo "No .env found. Run: cp .env.example .env  (then set POSTGRES_PASSWORD and ENCRYPTION_KEY)"
  exit 1
fi

env_value() { grep -E "^$1=" "$ROOT/.env" | tail -1 | cut -d= -f2- | tr -d '"' || true; }
if [ "$DEV_LOGIN" = false ] && [ -z "$(env_value GOOGLE_CLIENT_ID)" ]; then
  cat <<EOF
Google sign-in is not configured in .env, so nobody could sign in through the tunnel.

Either add GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET to .env (docs/google-oauth.md),
or run with --dev-login to sign in with any email address (read its warning first).
EOF
  exit 1
fi

mkdir -p "$STATE"

restore_local() {
  echo
  echo "› Closing the tunnel"
  [ -n "${TUNNEL_PID:-}" ] && kill "$TUNNEL_PID" 2>/dev/null || true
  [ -n "${AWAKE_PID:-}" ] && kill "$AWAKE_PID" 2>/dev/null || true
  if [ "${STARTED:-false}" = true ]; then
    echo "› Switching OncallKit back to http://localhost:$PORT"
    "${COMPOSE[@]}" up -d >/dev/null 2>&1 || true
  fi
  rm -f "$OVERRIDE"
}
trap restore_local EXIT
trap 'exit 130' INT TERM

echo "› Opening a Cloudflare quick tunnel to localhost:$PORT"
: > "$STATE/cloudflared.log"
cloudflared tunnel --no-autoupdate --protocol "${TUNNEL_PROTOCOL:-http2}" --url "http://localhost:$PORT" > "$STATE/cloudflared.log" 2>&1 &
TUNNEL_PID=$!

URL=""
for _ in $(seq 1 60); do
  URL="$(grep -oE 'https://[a-z0-9-]+\.trycloudflare\.com' "$STATE/cloudflared.log" | head -1 || true)"
  [ -n "$URL" ] && break
  kill -0 "$TUNNEL_PID" 2>/dev/null || break
  sleep 1
done
if [ -z "$URL" ]; then
  echo "Could not get a tunnel address. Log: $STATE/cloudflared.log"
  exit 1
fi

# The tunnel address becomes the public URL. Values here override .env for this run only.
cat > "$OVERRIDE" <<EOF
services:
  api:
    environment:
      PUBLIC_URL: "$URL"
      AUTH_DEV_LOGIN: "$DEV_LOGIN"
      TRUST_PROXY_HEADERS: "true"
EOF

echo "› Starting OncallKit for $URL (the first build takes a few minutes)"
STARTED=true
"${COMPOSE[@]}" -f "$OVERRIDE" up -d --build > "$STATE/compose.log" 2>&1 || {
  echo "Docker Compose failed. Log: $STATE/compose.log"
  exit 1
}

for _ in $(seq 1 60); do
  curl -fs "http://localhost:$PORT/readyz" >/dev/null 2>&1 && break
  sleep 1
done
curl -fs "http://localhost:$PORT/readyz" >/dev/null || {
  echo "OncallKit did not become ready. Logs: docker compose logs api"
  exit 1
}

echo "› Waiting for the tunnel to reach it"
for _ in $(seq 1 30); do
  curl -fs "$URL/healthz" >/dev/null 2>&1 && break
  sleep 2
done

# Keep the Mac awake while the tunnel is open (no-op elsewhere).
if command -v caffeinate >/dev/null; then
  caffeinate -dimsu -w "$TUNNEL_PID" &
  AWAKE_PID=$!
fi

cat <<EOF

  OncallKit is online at:

    $URL

EOF
if [ "$DEV_LOGIN" = true ]; then
  cat <<EOF
  ⚠  Development sign-in is ON: anyone who has this address can sign in as
     any email, including yours. Share it only with people you trust, and
     close the tunnel (Ctrl+C) when you are done.

EOF
else
  cat <<EOF
  Google sign-in: add this redirect URI to your OAuth client first
  (Google Cloud console → Credentials → your client → Authorised redirect URIs):

    $URL/api/v1/auth/google/callback

EOF
fi
cat <<EOF
  Open OncallKit at that address (not localhost) so sign-in cookies and links
  in Discord messages use it. Logs: docker compose logs -f api
  Press Ctrl+C to close the tunnel.

EOF

wait "$TUNNEL_PID"
