use std::process::ExitCode;

use cadence::cli::{Cli, Command};
use cadence::config::Config;
use cadence::{AppState, app, db, telemetry};
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
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { bind, database } => {
            let config = Config::new(&bind)?;
            telemetry::init();

            let pool = db::connect(&database).await?;
            db::migrate(&pool).await?;

            let listener = tokio::net::TcpListener::bind(config.bind).await?;
            tracing::info!(address = %config.bind, "cadence listening");

            axum::serve(listener, app(AppState { pool }))
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
        Command::Migrate { database } => {
            telemetry::init();

            let pool = db::connect(&database).await?;
            db::migrate(&pool).await?;
            tracing::info!(database = %database, "migrations applied");
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
