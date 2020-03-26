use crate::{
	config::{AuthMethod, SmtpDestinationConfig},
	hub::{MailAgent, MailDestination, Mail}
};
use std::{thread, sync::mpsc};
use log::{info, trace, error};
use native_tls::{TlsConnector};
use lettre::{
	SmtpClient, Transport, EmailAddress, Envelope,
    smtp::{
		ClientSecurity,
		ConnectionReuseParameters,
        authentication as auth,
        client::net::ClientTlsParameters
	}
};





enum SmtpDestinationMessage {
	Shutdown,
	Message { mail: Mail }
}

pub struct SmtpDestination {
	log_target: String,
	config: SmtpDestinationConfig,
	worker: Option<thread::JoinHandle<()>>,
	chan_send: mpsc::Sender<SmtpDestinationMessage>,
	chan_recv: Option<mpsc::Receiver<SmtpDestinationMessage>>
}
impl SmtpDestination {
	pub fn new(name: String, config: &SmtpDestinationConfig) -> Self {
		let (chan_send, chan_recv) = mpsc::channel();
		Self {
			log_target: format!("Smtp[{}]", name),
			config: config.clone(),
			worker: None,
			chan_send, chan_recv: Some(chan_recv)
		}
	}
}
impl MailAgent for SmtpDestination {
    fn start(&mut self) {
		info!(target: &self.log_target, "Starting");
		trace!(target: &self.log_target, "Using Configuration:\n{:?}", self.config);

		let log_target = self.log_target.clone();
		let config = self.config.clone();
		let chan_recv = self.chan_recv.take().unwrap();
		self.worker = Some(thread::spawn(move || {
			// construct connection security settings
			let mut tls_builder = TlsConnector::builder();
			tls_builder.min_protocol_version(Some(native_tls::Protocol::Tlsv11));
			let tls_parameters = ClientTlsParameters::new(config.server.clone(), tls_builder.build().unwrap());
			let security_settings = match config.ssl {
				true => ClientSecurity::Wrapper(tls_parameters),
				false => ClientSecurity::Required(tls_parameters)
			};
			
			// construct smtp client
			let mut mailer = SmtpClient::new(
				(config.server.as_str(), config.port),
				security_settings
			).expect("Failed to initialize smtp client")
			.connection_reuse(ConnectionReuseParameters::ReuseUnlimited);

			// configure authentication
			if let Some(auth) = config.auth {
				match auth {
					AuthMethod::Plain { user, password } => {
						mailer = mailer.credentials(auth::Credentials::new(user, password))
										.authentication_mechanism(auth::Mechanism::Plain);
					},
					AuthMethod::Login { user, password } => {
						mailer = mailer.credentials(auth::Credentials::new(user, password))
										.authentication_mechanism(auth::Mechanism::Login);
					},
				}
			}

			let mut mailer = mailer.transport();
			let recipient = EmailAddress::new(config.recipient).unwrap();

			while let Ok(msg) = chan_recv.recv() {
				match msg {
					SmtpDestinationMessage::Shutdown => return,
					SmtpDestinationMessage::Message{mail} => {
						// Construct mail
						let mail = lettre::Email::new(
							Envelope::new(None,vec![recipient.clone()]).unwrap(),
							"".to_owned(),
							mail.data
						);
						// Send mail
						match mailer.send(mail) {
							Ok(_) => info!(target: &log_target, "Successfully sent mail"),
							Err(err) => error!(target: &log_target, "Error while sending mail:\n{}", err)
						}
					}
				}
			}
		}));
	}
    fn stop(&mut self) {
		info!(target: &self.log_target, "Stopping");
		self.chan_send.send(SmtpDestinationMessage::Shutdown).unwrap();
		self.worker.take().unwrap().join().unwrap();
		info!(target: &self.log_target, "Stopped");
	}
}
impl MailDestination for SmtpDestination {
	
	fn enqueue(&mut self, mail: crate::hub::Mail) {
		self.chan_send.send(SmtpDestinationMessage::Message{ mail }).unwrap();
	}

}