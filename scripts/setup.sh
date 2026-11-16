#!/usr/bin/env bash
# One-shot developer setup: checks tools, installs the Diesel CLI,
# creates .env and initialises the database.
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust is not installed. Install it from https://rustup.rs and re-run." >&2
  exit 1
fi

if ! command -v diesel >/dev/null 2>&1; then
  echo "==> Installing Diesel CLI (SQLite only, bundled libsqlite3)…"
  cargo install diesel_cli --no-default-features --features sqlite-bundled
fi

if [ ! -f .env ]; then
  echo "==> Creating .env from .env.example"
  cp .env.example .env
fi

./scripts/init_db.sh

echo "==> Building"
cargo build --release

cat <<MSG

Setup complete.  Start the server with:

    cargo run --release

then open the BASE_URL from .env (default http://localhost:8080) and
register — the first account becomes the administrator.
MSG
