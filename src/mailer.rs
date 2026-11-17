//! Outgoing email. Used for address verification now and intended for
//! status-update notifications later (see THEORY_OF_OPERATION.md §16).

use std::sync::{Arc, Mutex};

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart},
    transport::smtp::authentication::Credentials,
};

use crate::config::MailConfig;

/// A rendered email ready to send.
#[derive(Clone, Debug)]
pub struct OutgoingEmail {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: String,
}

#[derive(Clone)]
pub enum Mailer {
    Smtp {
        transport: AsyncSmtpTransport<Tokio1Executor>,
        from: Mailbox,
    },
    Log,
    Memory(Arc<Mutex<Vec<OutgoingEmail>>>),
}

impl Mailer {
    pub fn from_config(cfg: &MailConfig) -> anyhow::Result<Self> {
        Ok(match cfg {
            MailConfig::Log => Mailer::Log,
            MailConfig::Memory => Mailer::Memory(Arc::default()),
            MailConfig::Smtp { host, port, security, username, password, from } => {
                let mut builder = match security.as_str() {
                    "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(host)?,
                    "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
                    _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)?,
                };
                if let Some(p) = port {
                    builder = builder.port(*p);
                }
                if let (Some(u), Some(p)) = (username, password) {
                    builder = builder.credentials(Credentials::new(u.clone(), p.clone()));
                }
                let from: Mailbox = from
                    .parse()
                    .map_err(|e| anyhow::anyhow!("MAIL_FROM {from:?} is not a valid address: {e}"))?;
                Mailer::Smtp { transport: builder.build(), from }
            }
        })
    }

    /// True when emails actually leave the server.
    pub fn delivers(&self) -> bool {
        matches!(self, Mailer::Smtp { .. })
    }

    pub async fn send(&self, email: OutgoingEmail) -> Result<(), String> {
        match self {
            Mailer::Log => {
                tracing::info!(
                    "email (SMTP not configured, not sent)\n  To: {}\n  Subject: {}\n{}",
                    email.to,
                    email.subject,
                    email.text
                );
                Ok(())
            }
            Mailer::Memory(outbox) => {
                outbox.lock().unwrap().push(email);
                Ok(())
            }
            Mailer::Smtp { transport, from } => {
                let to: Mailbox = email.to.parse().map_err(|e| format!("bad recipient: {e}"))?;
                let msg = Message::builder()
                    .from(from.clone())
                    .to(to)
                    .subject(email.subject)
                    .multipart(MultiPart::alternative_plain_html(email.text, email.html))
                    .map_err(|e| e.to_string())?;
                transport.send(msg).await.map(|_| ()).map_err(|e| {
                    tracing::error!("SMTP send failed: {e}");
                    e.to_string()
                })
            }
        }
    }

    /// Emails captured by the in-memory mailer (tests).
    pub fn sent(&self) -> Vec<OutgoingEmail> {
        match self {
            Mailer::Memory(outbox) => outbox.lock().unwrap().clone(),
            _ => Vec::new(),
        }
    }
}
