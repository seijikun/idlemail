use crate::{
    config::{AuthMethod, SmtpDestinationConfig},
    hub::{DestinationMessage, HubDestinationChannel, MailAgent, MailDestination},
};
use lettre::{
    address::Address as EmailAddress,
    transport::smtp::{
        authentication as auth, client::net::ClientTlsParameters, ClientSecurity,
        ConnectionReuseParameters,
    },
    Envelope, SmtpClient, Transport,
};
use log::{error, info, trace};
use native_tls::TlsConnector;
use std::thread;

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
        let recipient: EmailAddress = match self.config.recipient.parse() {
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
            // construct connection security settings
            let mut tls_builder = TlsConnector::builder();
            tls_builder.min_protocol_version(Some(native_tls::Protocol::Tlsv11));
            let tls_parameters =
                ClientTlsParameters::new(config.server.clone(), tls_builder.build().unwrap());
            let security_settings = if config.ssl {
                ClientSecurity::Wrapper(tls_parameters)
            } else {
                ClientSecurity::Required(tls_parameters)
            };

            // construct smtp client
            let mut mailer =
                SmtpClient::new((config.server.as_str(), config.port), security_settings)
                    .expect("Failed to initialize smtp client")
                    .connection_reuse(ConnectionReuseParameters::ReuseUnlimited);

            // configure authentication
            if let Some(auth) = config.auth {
                match auth {
                    AuthMethod::Plain { user, password } => {
                        mailer = mailer
                            .credentials(auth::Credentials::new(user, password))
                            .authentication_mechanism(auth::Mechanism::Plain);
                    }
                    AuthMethod::Login { user, password } => {
                        mailer = mailer
                            .credentials(auth::Credentials::new(user, password))
                            .authentication_mechanism(auth::Mechanism::Login);
                    }
                }
            }

            let mut mailer = mailer.transport();

            while let Ok(msg) = channel.next() {
                match msg {
                    DestinationMessage::Shutdown => {
                        info!(target: &log_target, "Stopping");
                        return;
                    }
                    DestinationMessage::Mail { mail } => {
                        // Send raw mail using constructed envelope
                        let evenlope = Envelope::new(None, vec![recipient.clone()]).unwrap();
                        match mailer.send_raw(&evenlope, &mail.data) {
                            Ok(_) => info!(target: &log_target, "Successfully sent mail"),
                            Err(err) => {
                                error!(target: &log_target, "Error while sending mail:\n{}", err);
                                channel.notify_failed_send(mail);
                            }
                        }
                    }
                }
            }
        }));
    }
}
