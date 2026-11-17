//! Greenhouse maintenance tracker. See `docs/THEORY_OF_OPERATION.md`.

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod mailer;
pub mod models;
pub mod routes;
pub mod sanitize;
pub mod schema;
pub mod uploads;
pub mod views;

use std::sync::Arc;

use axum::{Router, extract::DefaultBodyLimit, middleware};
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::{config::Config, db::DbPool};

#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub config: Arc<Config>,
    pub http: reqwest::Client,
    pub mailer: mailer::Mailer,
}

impl AppState {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let pool = db::build_pool(&config.database_url)?;
        if config.auto_migrate {
            for m in db::run_migrations(&pool)? {
                tracing::info!("applied migration {m}");
            }
        }
        std::fs::create_dir_all(&config.upload_dir)?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let mailer = mailer::Mailer::from_config(&config.mail)?;
        Ok(AppState {
            mailer,
            pool,
            config: Arc::new(config),
            http,
        })
    }
}

/// Build the complete HTTP application.
pub fn build_app(state: AppState) -> Router {
    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/static").to_string()
    });
    let body_limit = state.config.max_request_bytes;
    routes::router()
        .nest_service("/static", ServeDir::new(static_dir))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::same_origin_guard,
        ))
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
