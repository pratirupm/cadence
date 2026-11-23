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
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_defaults_to_loopback() {
        let cli = Cli::try_parse_from(["cadence", "serve"]).unwrap();

        assert_eq!(
            cli.command,
            Command::Serve {
                bind: "127.0.0.1:3000".to_owned()
            }
        );
    }

    #[test]
    fn serve_accepts_a_custom_address() {
        let cli = Cli::try_parse_from(["cadence", "serve", "--bind", "0.0.0.0:8080"]).unwrap();

        assert_eq!(
            cli.command,
            Command::Serve {
                bind: "0.0.0.0:8080".to_owned()
            }
        );
    }

    #[test]
    fn rejects_an_unknown_subcommand() {
        assert!(Cli::try_parse_from(["cadence", "frobnicate"]).is_err());
    }
}
