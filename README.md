# Greenhouse Maintenance Tracker

A mobile-friendly web app for reporting and tracking greenhouse maintenance
requests (broken pumps, misters, vents, heaters, lights…). It is written in Rust
(Axum + Askama + Diesel) and uses SQLite by default.

* Sign up with email and password, or with Google, Microsoft, GitHub, GitLab,
  Facebook, Discord, or any OpenID Connect provider.
* The dashboard shows the requests you submitted, the requests assigned to you,
  and the requests nobody has taken yet.
* Each request has a rich-text description, a submission timestamp, a priority,
  a location, a state (`unassigned`, `assigned`, `open` or `closed`), a
  submitter and an assignee.
* Each request has a conversation where the submitter and the assignee post
  questions and responses, with photos if needed.
* On a phone you can take photos with the camera and scan the barcode or QR
  code on a piece of equipment.

For an explanation of how the code works, see
[`docs/THEORY_OF_OPERATION.md`](docs/THEORY_OF_OPERATION.md).

---

## Initial setup

### 1. Prerequisites

| Tool | Why | Install |
|---|---|---|
| Rust 1.85+ (edition 2024) | builds the server | <https://rustup.rs> |
| C compiler (`cc`/`gcc`/`clang`) | builds the bundled SQLite | `apt install build-essential` / Xcode CLT |
| Diesel CLI *(optional, recommended)* | creates and migrates the database | installed by `scripts/setup.sh` |

You do not need a system SQLite library, because SQLite is compiled into the
binary.

### 2. One-command setup

```bash
git clone <this repo> tracker
```

```bash
cd tracker
```

```bash
./scripts/setup.sh
```

The setup script does four things:

1. Installs `diesel_cli` (SQLite only) if it is missing.
2. Copies `.env.example` to `.env`.
3. Runs `scripts/init_db.sh`, which creates `greenhouse.db` and applies all
   migrations.
4. Builds a release binary.

### 3. Run

```bash
cargo run --release
```

Open <http://localhost:8080> and **register**. **The first account created
becomes the administrator.** Administrators can grant or revoke admin rights to
other accounts on the *Users* page. You can also do it from the command line:

```bash
cargo run --release -- make-admin someone@example.com
```

### 4. Use it from phones (HTTPS)

Browsers only allow camera access from a page served over **HTTPS** (or from
`localhost`), and live barcode scanning needs the camera. Taking photos still
works over plain HTTP, through the file picker. For phones, use one of these:

* Put the app behind a reverse proxy that terminates TLS, such as Caddy,
  nginx or Traefik. With Caddy this is one line:
  `tracker.example.com { reverse_proxy localhost:8080 }`.
* For a quick test on your LAN, use a tunnel such as `cloudflared tunnel --url
  http://localhost:8080` or `tailscale serve 8080`.

Set `BASE_URL` in `.env` to the public `https://…` address. OAuth redirect URIs
are built from it, and session cookies get the `Secure` flag when it starts
with `https://`.

You can add the site to a phone's home screen. It ships a web-app manifest,
so it opens full-screen like an app.

---

## Configuration (`.env`)

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | `greenhouse.db` | SQLite file path |
| `BIND_ADDR` | `0.0.0.0:8080` | listen address |
| `BASE_URL` | `http://localhost:8080` | public URL (OAuth redirects, secure cookies) |
| `UPLOAD_DIR` | `uploads` | where photos are stored |
| `MAX_UPLOAD_MB` / `MAX_REQUEST_MB` | `15` / `60` | size limit per photo / per form post |
| `SESSION_HOURS` | `336` | how long a login lasts (default 14 days) |
| `ALLOW_REGISTRATION` | `true` | allow self sign-up (password and OAuth) |
| `AUTO_MIGRATE` | `true` | apply pending migrations when the server starts |
| `STATIC_DIR` | `<crate>/static` | location of CSS/JS (set this when deploying the binary elsewhere) |
| `RUST_LOG` | `greenhouse_tracker=info,tower_http=info` | log filter |

## Sign-in providers (OAuth)

Each provider's button appears once you set its `*_CLIENT_ID` **and**
`*_CLIENT_SECRET`. Register this redirect URI in the provider's console:

```
<BASE_URL>/auth/<provider>/callback
```

| Provider | `<provider>` | Env prefix | Where to create credentials | Notes |
|---|---|---|---|---|
| Google | `google` | `GOOGLE_` | Google Cloud Console → APIs & Services → Credentials → OAuth client ID (Web) | |
| Microsoft (Outlook, Microsoft 365, Entra ID) | `microsoft` | `MICROSOFT_` | Microsoft Entra admin center → App registrations | `MICROSOFT_TENANT` = `common` (default), `organizations`, `consumers` or a tenant id |
| GitHub | `github` | `GITHUB_` | GitHub → Settings → Developer settings → OAuth Apps | uses the primary *verified* email |
| GitLab (gitlab.com or self-hosted) | `gitlab` | `GITLAB_` | GitLab → Preferences → Applications (scopes `openid email profile`) | `GITLAB_BASE_URL` for self-hosted |
| Facebook | `facebook` | `FACEBOOK_` | developers.facebook.com → My Apps → Facebook Login | needs the `email` permission |
| Discord | `discord` | `DISCORD_` | discord.com/developers/applications → OAuth2 | |
| Any OpenID Connect provider (Okta, Auth0, Keycloak, Authentik, Zitadel, Azure AD B2C, …) | `oidc` | `OIDC_` | your identity provider | also set `OIDC_AUTH_URL`, `OIDC_TOKEN_URL`, `OIDC_USERINFO_URL`, `OIDC_DISPLAY_NAME` |

Any provider's scopes can be overridden with `<PREFIX>_SCOPES`, and its button
label with `<PREFIX>_DISPLAY_NAME`.

*Sign in with Apple* is not built in. It needs a JWT client secret signed with
an Apple private key and a `form_post` callback. See the theory-of-operation
doc for where to add it.

**Account linking.** Signing in with a provider for the first time creates a new
account. If that email address already has a password account, the sign-in is
refused rather than linked automatically. Instead, the user logs in with the
password and clicks **Link** on the *Profile* page.

---

## Database scripts

| Command | What it does |
|---|---|
| `scripts/init_db.sh` | Creates the database if missing and applies all migrations |
| `scripts/init_db.sh --reset` | **Deletes** the database and uploads, then recreates them |
| `scripts/migrate.sh run` | Backs up the database, then applies pending migrations |
| `scripts/migrate.sh status` | Lists migrations and whether each one is applied |
| `scripts/migrate.sh revert` / `redo` | Rolls back the latest migration (`redo` re-applies it) |
| `scripts/migrate.sh new add_due_date` | Creates `migrations/<timestamp>_add_due_date/{up,down}.sql` |
| `scripts/migrate.sh schema` | Regenerates `src/schema.rs` from the database |
| `scripts/migrate.sh backup` | Copies the database to `backups/` |
| `greenhouse-tracker migrate` | Applies migrations using the copy embedded in the binary (no Diesel CLI needed on servers) |

The workflow for a schema change is: `migrate.sh new <name>`, then write
`up.sql` and `down.sql`, then `migrate.sh run` (which also updates
`src/schema.rs`), then update `src/models.rs`, then `migrate.sh redo` to test the
rollback.

## Other commands

```bash
cargo test
```

```bash
cargo run -- create-user tech@example.com "Pat Tech"
```

`cargo test` runs the end-to-end tests: registration, issues, conversation,
photos and OAuth against a mock provider. `create-user` creates a password
account and reads the password from stdin.

## Deploying

```bash
cargo build --release
```

Copy `target/release/greenhouse-tracker`, the `static/` directory and a `.env`
to the server. Set `STATIC_DIR` to the path of `static/`, then run the binary
under systemd or similar. Templates and migrations are compiled into the
binary. To back up, copy the SQLite file (use `sqlite3 db ".backup file"` while
the server is running) and the `uploads/` directory.

## Third-party code

`static/vendor/zxing-library.min.js` is
[ZXing for JavaScript](https://github.com/zxing-js/library) 0.21.3, under the
Apache-2.0 license (`static/vendor/ZXING-LICENSE`). Browsers without a native
`BarcodeDetector`, such as iOS Safari and Firefox, use it for barcode scanning.
