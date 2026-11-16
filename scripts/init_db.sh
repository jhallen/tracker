#!/usr/bin/env bash
# Create the SQLite database (if missing) and apply all migrations.
#
#   scripts/init_db.sh            create / bring up to date
#   scripts/init_db.sh --reset    DELETE the database and start over
#
# Uses the Diesel CLI when installed; otherwise falls back to the
# migrations embedded in the server binary (`greenhouse-tracker migrate`).
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f .env ] && set -a && . ./.env && set +a
export DATABASE_URL="${DATABASE_URL:-greenhouse.db}"
UPLOAD_DIR="${UPLOAD_DIR:-uploads}"

if [ "${1:-}" = "--reset" ]; then
  read -r -p "This deletes $DATABASE_URL and all uploaded photos in $UPLOAD_DIR. Continue? [y/N] " ok
  [ "$ok" = "y" ] || [ "$ok" = "Y" ] || exit 1
  rm -f "$DATABASE_URL" "$DATABASE_URL-wal" "$DATABASE_URL-shm"
  rm -rf "$UPLOAD_DIR"
fi

mkdir -p "$UPLOAD_DIR"

if command -v diesel >/dev/null 2>&1; then
  echo "==> diesel setup ($DATABASE_URL)"
  # `setup` creates the database and runs all pending migrations.
  # It also regenerates src/schema.rs (see diesel.toml).
  diesel setup
else
  echo "==> Diesel CLI not found; using embedded migrations"
  cargo run --quiet -- migrate
fi

echo "==> Database ready: $DATABASE_URL"
