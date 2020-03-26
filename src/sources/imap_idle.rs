use crate::{
	config::ImapIdleSourceConfig,
	hub::{MailAgent, MailSource, MailChannel}
};
use super::common::ImapConnection;
use log::{info, debug, trace};
use async_std::task;
use futures::{future::FutureExt, pin_mut, select};
use std::{time::Duration, thread};

pub struct ImapIdleSource {
	log_target: String,
	config: ImapIdleSourceConfig,
	worker: Option<thread::JoinHandle<()>>,
	mail_channel: Option<MailChannel>,
	stop_sender: Option<async_std::sync::Sender<()>>
}
impl ImapIdleSource {
	pub fn new(name: String, config: &ImapIdleSourceConfig, mail_channel: MailChannel) -> Self {
		Self {
			log_target: format!("ImapIdle[{}]", name),
			config: config.clone(),
			worker: None,
			mail_channel: Some(mail_channel),
			stop_sender: None
		}
	}
}
impl MailAgent for ImapIdleSource {
    fn start(&mut self) {
		info!(target: &self.log_target, "Starting");
		trace!(target: &self.log_target, "Using Configuration:\n{:?}", self.config);

		let log_target = self.log_target.clone();
		let config = self.config.clone();
		let (stop_sender, stop_recv) = async_std::sync::channel::<()>(1);
		self.stop_sender = Some(stop_sender);
		let mail_channel = self.mail_channel.take().unwrap();

		self.worker = Some(thread::spawn(move || {
			let mut con = ImapConnection::new(
				config.server.clone(), config.port, config.auth.clone());

			let stop_future = stop_recv.recv().fuse();
			pin_mut!(stop_future);

			loop {
				con.iter_unseen_recursive(Some(&config.path), !config.keep).unwrap().for_each(|unseen_message| {
					if let Ok( (path, unseen_message) ) = unseen_message {
						debug!(target: &log_target, "Unread mail in {}", path);
						mail_channel.notify(unseen_message);
					}
				});

				info!(target: &log_target, "Entering IMAP IDLE to wait for server notification");
				con.run(|sess| task::block_on(sess.select(config.path.clone()))).unwrap();
				let mut idle_handle = con.idle().unwrap();
				// Dropping the StopSource interrupts idle, the variable thus needs a name.
				let (idle_future, _stopsrc) = idle_handle.wait_with_timeout(Duration::from_secs(config.renewinterval));

				// await either a wake-up from the IMAP server, or a request to shutdown
				let idle_future = idle_future.fuse();
				pin_mut!(idle_future);
				let should_exit = task::block_on(async {
					loop {
						select! {
							_ = idle_future => return false,
							_ = stop_future => return true,
							complete => unreachable!()
						};
					}
				});
				if should_exit { return; }
				debug!(target: &log_target, "IDLE interrupted");
			}
		}));
	}

    fn stop(&mut self) {
		info!(target: &self.log_target, "Stopping");
		task::block_on(self.stop_sender.as_mut().unwrap().send(()));
		self.worker.take().unwrap().join().unwrap();
		info!(target: &self.log_target, "Stopped");
	}
}
impl MailSource for ImapIdleSource {}