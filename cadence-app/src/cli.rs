use clap::{Parser, Subcommand};

/// The Cadence task service.
#[derive(Debug, Parser)]
#[command(name = "cadence", version, about = "A multi-user task service")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Start the HTTP server.
    Serve {
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:3000")]
        bind: String,
        /// SQLite database path or URL.
        #[arg(long, env = "CADENCE_DATABASE", default_value = "cadence.db")]
        database: String,
    },
    /// Apply pending database migrations.
    Migrate {
        /// SQLite database path or URL.
        #[arg(long, env = "CADENCE_DATABASE", default_value = "cadence.db")]
        database: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_defaults_to_loopback_and_a_local_database() {
        let cli = Cli::try_parse_from(["cadence", "serve"]).unwrap();

        assert_eq!(
            cli.command,
            Command::Serve {
                bind: "127.0.0.1:3000".to_owned(),
                database: "cadence.db".to_owned(),
            }
        );
    }

    #[test]
    fn serve_accepts_a_custom_address_and_database() {
        let cli = Cli::try_parse_from([
            "cadence",
            "serve",
            "--bind",
            "0.0.0.0:8080",
            "--database",
            "test.db",
        ])
        .unwrap();

        assert_eq!(
            cli.command,
            Command::Serve {
                bind: "0.0.0.0:8080".to_owned(),
                database: "test.db".to_owned(),
            }
        );
    }

    #[test]
    fn migrate_accepts_a_database() {
        let cli = Cli::try_parse_from(["cadence", "migrate", "--database", "test.db"]).unwrap();

        assert_eq!(
            cli.command,
            Command::Migrate {
                database: "test.db".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_an_unknown_subcommand() {
        assert!(Cli::try_parse_from(["cadence", "frobnicate"]).is_err());
    }
}
