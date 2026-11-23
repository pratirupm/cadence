use std::sync::{Arc, Mutex};

use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("could not send email: {0}")]
    Send(String),
}

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, message: &EmailMessage) -> Result<(), MailError>;
}

#[derive(Debug, Default, Clone)]
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, message: &EmailMessage) -> Result<(), MailError> {
        tracing::info!(
            to = %message.to,
            subject = %message.subject,
            "email delivered to the log"
        );
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct RecordingMailer {
    messages: Arc<Mutex<Vec<EmailMessage>>>,
}

impl RecordingMailer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(&self) -> Vec<EmailMessage> {
        self.messages.lock().expect("mailbox lock").clone()
    }
}

#[async_trait]
impl Mailer for RecordingMailer {
    async fn send(&self, message: &EmailMessage) -> Result<(), MailError> {
        self.messages
            .lock()
            .expect("mailbox lock")
            .push(message.clone());
        Ok(())
    }
}

pub fn password_reset_email(to: &str, token: &str) -> EmailMessage {
    EmailMessage {
        to: to.to_owned(),
        subject: "Reset your Cadence password".to_owned(),
        text: format!(
            "Open /reset/{token} to choose a new password. The link expires in one hour."
        ),
    }
}

pub fn verification_email(to: &str, token: &str) -> EmailMessage {
    EmailMessage {
        to: to.to_owned(),
        subject: "Verify your Cadence email".to_owned(),
        text: format!("Open /verify/{token} to verify your email address."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_messages_for_tests() {
        let mailer = RecordingMailer::new();
        let message = verification_email("person@example.com", "token");

        mailer.send(&message).await.unwrap();

        assert_eq!(mailer.messages(), [message]);
    }

    #[test]
    fn reset_email_contains_the_token() {
        let message = password_reset_email("person@example.com", "abc123");

        assert!(message.text.contains("/reset/abc123"));
    }
}
