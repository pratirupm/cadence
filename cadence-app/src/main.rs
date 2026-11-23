use std::process::ExitCode;

use cadence::auth::AuthService;
use cadence::cli::{Cli, Command, UserCommand};
use cadence::config::Config;
use cadence::reminders::{ReminderService, run_forever};
use cadence::{AppState, app, db, telemetry};
use chrono::Utc;
use clap::Parser;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let Cli { database, command } = Cli::parse();
    telemetry::init();

    match command {
        Command::Serve { bind } => {
            let config = Config::new(&bind)?;

            let pool = db::connect(&database).await?;
            db::migrate(&pool).await?;

            let listener = tokio::net::TcpListener::bind(config.bind).await?;
            tracing::info!(address = %config.bind, "cadence listening");

            axum::serve(listener, app(AppState::new(pool)))
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
        Command::Migrate => {
            let pool = db::connect(&database).await?;
            db::migrate(&pool).await?;
            tracing::info!(database = %database, "migrations applied");
        }
        Command::Worker { once } => {
            let pool = db::connect(&database).await?;
            db::migrate(&pool).await?;

            if once {
                let count = ReminderService::new(pool).run_once(Utc::now(), 50).await?;
                println!("sent {count} reminder(s)");
            } else {
                tokio::select! {
                    result = run_forever(pool) => result?,
                    () = shutdown_signal() => {},
                }
            }
        }
        Command::User { command } => {
            let pool = db::connect(&database).await?;
            db::migrate(&pool).await?;
            let auth = AuthService::new(pool);

            match command {
                UserCommand::Create {
                    email,
                    name,
                    password,
                    verified,
                } => {
                    let user = auth.create_user(&email, &name, &password, verified).await?;
                    println!("{} {}", user.id, user.email);
                }
            }
        }
    }

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
