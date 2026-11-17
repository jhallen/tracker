//! Runtime configuration, read from environment variables (and `.env`).

use std::{env, path::PathBuf};

use crate::auth::oauth::{OAuthProvider, ProviderKind};

#[derive(Clone, Debug)]
pub struct Config {
    /// Diesel/SQLite database path, e.g. `greenhouse.db`.
    pub database_url: String,
    /// Address to bind, e.g. `0.0.0.0:8080`.
    pub bind_addr: String,
    /// Public URL users reach the app at (used for OAuth redirect URIs and
    /// to decide whether cookies get the `Secure` flag).
    pub base_url: String,
    /// Directory where uploaded photos are stored.
    pub upload_dir: PathBuf,
    /// Max size of one uploaded file, in bytes.
    pub max_upload_bytes: usize,
    /// Max size of a whole request body (all photos together), in bytes.
    pub max_request_bytes: usize,
    /// Session lifetime in hours.
    pub session_hours: i64,
    /// Whether new users may self-register.
    pub allow_registration: bool,
    /// Run pending migrations automatically at startup.
    pub auto_migrate: bool,
    /// OAuth providers that have credentials configured.
    pub oauth_providers: Vec<OAuthProvider>,
    /// How outgoing email is delivered.
    pub mail: MailConfig,
}

/// Outgoing email settings. Without `SMTP_HOST` the app still runs and
/// writes each email (including verification links) to the server log.
#[derive(Clone, Debug)]
pub enum MailConfig {
    Smtp {
        host: String,
        port: Option<u16>,
        /// `starttls` (default, port 587), `tls` (implicit TLS, port 465)
        /// or `none` (plain, for a local relay or a test catcher like Mailpit).
        security: String,
        username: Option<String>,
        password: Option<String>,
        from: String,
    },
    /// Log emails instead of sending them (development default).
    Log,
    /// Keep emails in memory (tests).
    Memory,
}

impl MailConfig {
    fn from_env() -> Self {
        let Some(host) = var("SMTP_HOST") else {
            return MailConfig::Log;
        };
        MailConfig::Smtp {
            host,
            port: var("SMTP_PORT").and_then(|p| p.parse().ok()),
            security: var("SMTP_SECURITY").unwrap_or_else(|| "starttls".into()).to_ascii_lowercase(),
            username: var("SMTP_USERNAME"),
            password: var("SMTP_PASSWORD"),
            from: var("MAIL_FROM")
                .unwrap_or_else(|| "Greenhouse Maintenance <noreply@localhost>".into()),
        }
    }
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn flag(name: &str, default: bool) -> bool {
    match var(name).as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("1" | "true" | "yes" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        _ => default,
    }
}

impl Config {
    pub fn from_env() -> Self {
        let base_url = var("BASE_URL")
            .unwrap_or_else(|| "http://localhost:8080".into())
            .trim_end_matches('/')
            .to_string();
        let mut oauth_providers = Vec::new();
        for kind in ProviderKind::ALL {
            if let Some(p) = OAuthProvider::from_env(*kind) {
                oauth_providers.push(p);
            }
        }
        Config {
            database_url: var("DATABASE_URL").unwrap_or_else(|| "greenhouse.db".into()),
            bind_addr: var("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:8080".into()),
            base_url,
            upload_dir: var("UPLOAD_DIR").unwrap_or_else(|| "uploads".into()).into(),
            max_upload_bytes: var("MAX_UPLOAD_MB")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(15)
                * 1024
                * 1024,
            max_request_bytes: var("MAX_REQUEST_MB")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(60)
                * 1024
                * 1024,
            session_hours: var("SESSION_HOURS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(24 * 14),
            allow_registration: flag("ALLOW_REGISTRATION", true),
            auto_migrate: flag("AUTO_MIGRATE", true),
            oauth_providers,
            mail: MailConfig::from_env(),
        }
    }

    /// Settings suitable for tests: in-memory-ish temp DB, no OAuth.
    pub fn for_tests(database_url: String, upload_dir: PathBuf) -> Self {
        Config {
            database_url,
            bind_addr: "127.0.0.1:0".into(),
            base_url: "http://localhost".into(),
            upload_dir,
            max_upload_bytes: 5 * 1024 * 1024,
            max_request_bytes: 20 * 1024 * 1024,
            session_hours: 1,
            allow_registration: true,
            auto_migrate: true,
            oauth_providers: Vec::new(),
            mail: MailConfig::Memory,
        }
    }

    pub fn secure_cookies(&self) -> bool {
        self.base_url.starts_with("https://")
    }

    pub fn provider(&self, slug: &str) -> Option<&OAuthProvider> {
        self.oauth_providers.iter().find(|p| p.kind.slug() == slug)
    }
}
