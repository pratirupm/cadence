use clap::{Parser, Subcommand};

/// The Cadence task service.
#[derive(Debug, Parser)]
#[command(name = "cadence", version, about = "A multi-user task service")]
pub struct Cli {
    /// SQLite database path or URL.
    #[arg(
        long,
        env = "CADENCE_DATABASE",
        default_value = "cadence.db",
        global = true
    )]
    pub database: String,
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
    },
    /// Apply pending database migrations.
    Migrate,
    /// Manage accounts.
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
pub enum UserCommand {
    /// Create an account without email verification.
    Create {
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: String,
        #[arg(long, env = "CADENCE_PASSWORD", hide_env_values = true)]
        password: String,
        /// Mark the email address as verified.
        #[arg(long)]
        verified: bool,
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
            }
        );
        assert_eq!(cli.database, "cadence.db");
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
            }
        );
        assert_eq!(cli.database, "test.db");
    }

    #[test]
    fn migrate_accepts_a_database() {
        let cli = Cli::try_parse_from(["cadence", "migrate", "--database", "test.db"]).unwrap();

        assert_eq!(cli.command, Command::Migrate);
        assert_eq!(cli.database, "test.db");
    }

    #[test]
    fn user_create_parses_account_details() {
        let cli = Cli::try_parse_from([
            "cadence",
            "user",
            "--database",
            "test.db",
            "create",
            "--email",
            "person@example.com",
            "--name",
            "Ada",
            "--password",
            "correct horse battery staple",
            "--verified",
        ])
        .unwrap();

        assert_eq!(
            cli.command,
            Command::User {
                command: UserCommand::Create {
                    email: "person@example.com".to_owned(),
                    name: "Ada".to_owned(),
                    password: "correct horse battery staple".to_owned(),
                    verified: true,
                },
            }
        );
        assert_eq!(cli.database, "test.db");
    }

    #[test]
    fn rejects_an_unknown_subcommand() {
        assert!(Cli::try_parse_from(["cadence", "frobnicate"]).is_err());
    }
}
