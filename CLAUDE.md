# Notes for Claude Code

Read `docs/THEORY_OF_OPERATION.md` first. It covers the architecture, the data
model, the request flows, the conventions, and the open TODO list.

Quick facts:
- Rust 2024, Axum 0.8, Askama 0.16 (templates in `templates/`, compiled in), Diesel 2.4 + SQLite (bundled).
- `cargo test` runs `tests/flow.rs` and `tests/oauth.rs` (in-process, no network). Keep them passing.
- Schema changes: `scripts/migrate.sh new <name>`, then edit SQL, then `scripts/migrate.sh run` (regenerates `src/schema.rs`), then update `src/models.rs`.
- Diesel calls are blocking. Always wrap them in `db::run(&state.pool, |conn| ...)`.
- Every non-GET request must pass `auth::same_origin_guard`. Tests send `Origin: http://localhost` and `Host: localhost`.
- Update the TODO / changelog section of the theory doc when you finish a piece of work.
