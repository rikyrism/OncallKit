#!/usr/bin/env sh
# Run cargo inside the official Rust image, for machines without a local Rust toolchain.
# Usage: scripts/cargo-docker.sh test --workspace
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec docker run --rm \
  -v "$ROOT":/work -w /work \
  -v oncallkit-cargo-registry:/usr/local/cargo/registry \
  -v oncallkit-target:/work/target \
  -e DATABASE_URL="${DATABASE_URL:-}" \
  -e SQLX_OFFLINE=true \
  --network "${CARGO_DOCKER_NETWORK:-bridge}" \
  rust:1-bookworm cargo "$@"
