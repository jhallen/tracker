//! Command-line entry point.
//!
//! ```text
//! greenhouse-tracker [serve]                     run the web server (default)
//! greenhouse-tracker migrate                     apply pending migrations and exit
//! greenhouse-tracker make-admin <email>          grant admin rights to a user
//! greenhouse-tracker create-user <email> <name>  create a password user (prompts on stdin)
//! ```

use diesel::prelude::*;
use greenhouse_tracker::{AppState, auth, build_app, config::Config, db, schema::users};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("greenhouse_tracker=info,tower_http=info")),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Config::from_env();

    match args.first().map(String::as_str) {
        None | Some("serve") => serve(config).await,
        Some("migrate") => {
            let pool = db::build_pool(&config.database_url)?;
            let applied = db::run_migrations(&pool)?;
            if applied.is_empty() {
                println!("Database is up to date.");
            }
            for m in applied {
                println!("Applied {m}");
            }
            Ok(())
        }
        Some("make-admin") => {
            let email = args.get(1).ok_or_else(|| anyhow::anyhow!("usage: make-admin <email>"))?;
            let pool = db::build_pool(&config.database_url)?;
            let mut conn = pool.get()?;
            let n = diesel::update(users::table.filter(users::email.eq(auth::normalize_email(email))))
                .set(users::is_admin.eq(true))
                .execute(&mut conn)?;
            if n == 0 {
                anyhow::bail!("no user with email {email}");
            }
            println!("{email} is now an admin.");
            Ok(())
        }
        Some("create-user") => {
            let (Some(email), Some(name)) = (args.get(1), args.get(2)) else {
                anyhow::bail!("usage: create-user <email> <display name>  (password read from stdin)");
            };
            let mut password = String::new();
            eprint!("Password: ");
            std::io::stdin().read_line(&mut password)?;
            let password = password.trim_end_matches(['\r', '\n']);
            if password.len() < auth::MIN_PASSWORD_LEN {
                anyhow::bail!("password must be at least {} characters", auth::MIN_PASSWORD_LEN);
            }
            let pool = db::build_pool(&config.database_url)?;
            db::run_migrations(&pool)?;
            let mut conn = pool.get()?;
            let hash = auth::hash_password(password).map_err(|e| anyhow::anyhow!("{e}"))?;
            let user = auth::create_user(&mut conn, &auth::normalize_email(email), name, Some(&hash))
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("Created user #{} ({}){}", user.id, user.email, if user.is_admin { " as admin" } else { "" });
            Ok(())
        }
        Some(other) => anyhow::bail!("unknown command {other:?}; try serve, migrate, make-admin, create-user"),
    }
}

async fn serve(config: Config) -> anyhow::Result<()> {
    let bind = config.bind_addr.clone();
    let providers: Vec<_> = config.oauth_providers.iter().map(|p| p.slug()).collect();
    let state = AppState::new(config)?;
    tracing::info!(
        "OAuth providers enabled: {}",
        if providers.is_empty() { "none".to_string() } else { providers.join(", ") }
    );
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(())
}
