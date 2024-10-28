use crate::{
    config::{AuthMethod, SmtpDestinationConfig},
    hub::{DestinationMessage, HubDestinationChannel, MailAgent},
};
use lettre::{
    address::Envelope, transport::smtp::authentication as auth, Address, SmtpTransport, Transport,
};
use log::{error, info, trace, warn};
use std::thread;

use super::MailDestination;

pub struct SmtpDestination {
    log_target: String,
    config: SmtpDestinationConfig,
    worker: Option<thread::JoinHandle<()>>,
}
impl SmtpDestination {
    pub fn new(name: String, config: &SmtpDestinationConfig) -> Self {
        Self {
            log_target: format!("Smtp[{}]", name),
            config: config.clone(),
            worker: None,
        }
    }
}
impl MailAgent for SmtpDestination {
    fn join(&mut self) {
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("Thread exited with errors");
    }
}
impl MailDestination for SmtpDestination {
    fn start(&mut self, channel: HubDestinationChannel) {
        info!(target: &self.log_target, "Starting");
        let log_target = self.log_target.clone();
        let recipient: Address = match self.config.recipient.parse() {
            Ok(recipient) => recipient,
            Err(err) => {
                error!(
                    target: &log_target,
                    "Configured recipient is not a valid mail address: {}", err
                );
                return;
            }
        };
        trace!(target: &self.log_target, "Using Configuration:\n{:?}", self.config);
        let config = self.config.clone();

        self.worker = Some(thread::spawn(move || {
            let mut connection_builder = match config.encryption {
                crate::config::Encryption::None => SmtpTransport::builder_dangerous(&config.server),
                crate::config::Encryption::Ssl => {
                    SmtpTransport::relay(&config.server).expect("Failed to initialize smtp client")
                }
                crate::config::Encryption::Starttls => {
                    SmtpTransport::starttls_relay(&config.server)
                        .expect("Failed to initialize smtp client")
                }
            };

            connection_builder = connection_builder.port(config.port);

            // configure authentication
            if let Some(auth) = config.auth {
                match auth {
                    AuthMethod::None => {}
                    AuthMethod::Plain { user, password } => {
                        connection_builder = connection_builder
                            .credentials(auth::Credentials::new(user, password))
                            .authentication(vec![auth::Mechanism::Plain]);
                    }
                    AuthMethod::Login { user, password } => {
                        connection_builder = connection_builder
                            .credentials(auth::Credentials::new(user, password))
                            .authentication(vec![auth::Mechanism::Login]);
                    }
                }
            }

            let mailer = connection_builder.build();

            while let Ok(DestinationMessage::Mail { mail }) = channel.next() {
                // Send raw mail using constructed envelope
                let evenlope = Envelope::new(None, vec![recipient.clone()]).unwrap();
                match mailer.send_raw(&evenlope, &mail.data) {
                    Ok(_) => info!(target: &log_target, "Successfully sent mail"),
                    Err(err) => {
                        if err.is_permanent() {
                            warn!(target: &log_target, "The destination server does not accept this email, will not try again:\n{}", err);
                        } else {
                            error!(target: &log_target, "Error while sending mail:\n{}", err);
                            channel.notify_failed_send(mail);
                        }
                    }
                }
            }
            info!(target: &log_target, "Stopping");
        }));
    }
}
