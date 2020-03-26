use crate::{
	config::ImapPollSourceConfig,
	hub::{MailAgent, MailSource, MailChannel}
};
use super::common::ImapConnection;
use log::{info, debug, trace};
use std::{time::Duration, thread, sync::mpsc};


pub struct ImapPollSource {
	log_target: String,
	config: ImapPollSourceConfig,
	worker: Option<thread::JoinHandle<()>>,
	mail_channel: Option<MailChannel>,
	stop_sender: Option<mpsc::Sender<()>>
}
impl ImapPollSource {
	pub fn new(name: String, config: &ImapPollSourceConfig, mail_channel: MailChannel) -> Self {
		Self {
			log_target: format!("ImapPoll[{}]", name),
			config: config.clone(),
			worker: None,
			mail_channel: Some(mail_channel),
			stop_sender: None
		}
	}
}
impl MailAgent for ImapPollSource {
    fn start(&mut self) {
		info!(target: &self.log_target, "Starting");
		trace!(target: &self.log_target, "Using Configuration:\n{:?}", self.config);

		let log_target = self.log_target.clone();
		let config = self.config.clone();
		let (stop_sender, stop_recv) = mpsc::channel::<()>();
		self.stop_sender = Some(stop_sender);
		let mail_channel = self.mail_channel.take().unwrap();

		self.worker = Some(thread::spawn(move || {
			let mut con = ImapConnection::new(
				config.server.clone(), config.port, config.auth);
			loop {
				info!(target: &log_target, "Polling for unread mails");
				con.iter_unseen_recursive(None, !config.keep).unwrap().for_each(|unseen_message| {
					if let Ok( (path, unseen_message) ) = unseen_message {
						debug!(target: &log_target, "Unread mail in {}", path);
						mail_channel.notify(unseen_message);
					}
				});

				// sleep until next poll is due - interrupt if requested to stop
				if let Ok(_) = stop_recv.recv_timeout(Duration::from_secs(config.interval)) {
					// received stop request -> return, otherwise, do next poll round
					return;
				}
			}
		}));
	}
    fn stop(&mut self) {
		info!(target: &self.log_target, "Stopping");
		self.stop_sender.take().unwrap().send(()).unwrap();
		self.worker.take().unwrap().join().unwrap();
		info!(target: &self.log_target, "Stopped");
	}
}
impl MailSource for ImapPollSource {}

//see: https://gitlab.com/fetchmail/fetchmail/-/blob/legacy_64/imap.c