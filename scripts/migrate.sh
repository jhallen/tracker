#!/usr/bin/env bash
# Schema migration helper (wraps the Diesel CLI).
#
#   scripts/migrate.sh run              apply pending migrations
#   scripts/migrate.sh status           list migrations and whether applied
#   scripts/migrate.sh revert           roll back the latest migration
#   scripts/migrate.sh redo             revert + re-run the latest (test down.sql)
#   scripts/migrate.sh new <name>       create migrations/<timestamp>_<name>/{up,down}.sql
#   scripts/migrate.sh schema           regenerate src/schema.rs from the database
#   scripts/migrate.sh backup           copy the database to backups/<timestamp>.db
#
# `run`, `revert` and `redo` regenerate src/schema.rs automatically.
# In production without the Diesel CLI, `greenhouse-tracker migrate`
# (or AUTO_MIGRATE=true) applies the same migrations, which are embedded
# in the binary at build time.
set -euo pipefail
cd "$(dirname "$0")/.."

[ -f .env ] && set -a && . ./.env && set +a
export DATABASE_URL="${DATABASE_URL:-greenhouse.db}"

need_diesel() {
  if ! command -v diesel >/dev/null 2>&1; then
    echo "Diesel CLI not found. Install with:" >&2
    echo "  cargo install diesel_cli --no-default-features --features sqlite-bundled" >&2
    exit 1
  fi
}

backup() {
  if [ -f "$DATABASE_URL" ]; then
    mkdir -p backups
    local dest="backups/$(basename "$DATABASE_URL" .db)-$(date +%Y%m%d-%H%M%S).db"
    if command -v sqlite3 >/dev/null 2>&1; then
      sqlite3 "$DATABASE_URL" ".backup '$dest'"
    else
      cp "$DATABASE_URL" "$dest"
    fi
    echo "==> Backed up to $dest"
  fi
}

cmd="${1:-run}"
case "$cmd" in
  run)    need_diesel; backup; diesel migration run ;;
  status) need_diesel; diesel migration list ;;
  revert) need_diesel; backup; diesel migration revert ;;
  redo)   need_diesel; backup; diesel migration redo ;;
  new)
    need_diesel
    name="${2:?usage: scripts/migrate.sh new <name>}"
    diesel migration generate "$name"
    echo "Edit the new up.sql / down.sql, then: scripts/migrate.sh run"
    ;;
  schema) need_diesel; diesel print-schema > src/schema.rs; echo "==> src/schema.rs updated" ;;
  backup) backup ;;
  *) sed -n '2,16p' "$0"; exit 1 ;;
esac
