use crate::{
    config::{AuthMethod, SmtpDestinationConfig},
    hub::{DestinationMessage, HubDestinationChannel, MailAgent, MailDestination},
};
use lettre::{
    smtp::{
        authentication as auth, client::net::ClientTlsParameters, ClientSecurity,
        ConnectionReuseParameters,
    },
    EmailAddress, Envelope, SmtpClient, Transport,
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
        trace!(target: &self.log_target, "Using Configuration:\n{:?}", self.config);

        let log_target = self.log_target.clone();
        let config = self.config.clone();
        self.worker = Some(thread::spawn(move || {
            // construct connection security settings
            let mut tls_builder = TlsConnector::builder();
            tls_builder.min_protocol_version(Some(native_tls::Protocol::Tlsv11));
            let tls_parameters =
                ClientTlsParameters::new(config.server.clone(), tls_builder.build().unwrap());
            let security_settings = match config.ssl {
                true => ClientSecurity::Wrapper(tls_parameters),
                false => ClientSecurity::Required(tls_parameters),
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
            let recipient = EmailAddress::new(config.recipient).unwrap();

            while let Ok(msg) = channel.next() {
                match msg {
                    DestinationMessage::Shutdown => {
                        info!(target: &log_target, "Stopping");
                        return;
                    }
                    DestinationMessage::Mail { mail } => {
                        // Construct mail
                        let enveloped_mail = lettre::Email::new(
                            Envelope::new(None, vec![recipient.clone()]).unwrap(),
                            "".to_owned(),
                            mail.data.clone(),
                        );
                        // Send mail
                        match mailer.send(enveloped_mail) {
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
