use std::net::SocketAddr;

use thiserror::Error;

/// Settings needed to start the HTTP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind: SocketAddr,
}

impl Config {
    /// Parse a bind address supplied by the command line.
    pub fn new(bind: &str) -> Result<Self, ConfigError> {
        let bind = bind
            .parse()
            .map_err(|_| ConfigError::InvalidBind(bind.to_owned()))?;
        Ok(Self { bind })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("invalid bind address: {0}")]
    InvalidBind(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_socket_address() {
        let config = Config::new("127.0.0.1:4000").unwrap();

        assert_eq!(config.bind, "127.0.0.1:4000".parse().unwrap());
    }

    #[test]
    fn rejects_an_invalid_bind_address() {
        let error = Config::new("not-an-address").unwrap_err();

        assert_eq!(error, ConfigError::InvalidBind("not-an-address".to_owned()));
    }
}
