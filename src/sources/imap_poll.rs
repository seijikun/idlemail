use super::common::ImapConnection;
use crate::{
    config::ImapPollSourceConfig,
    hub::{HubSourceChannel, MailAgent, MailSource},
};
use log::{debug, info, trace};
use std::{thread, time::Duration};

pub struct ImapPollSource {
    log_target: String,
    config: ImapPollSourceConfig,
    worker: Option<thread::JoinHandle<()>>,
}
impl ImapPollSource {
    pub fn new(name: String, config: &ImapPollSourceConfig) -> Self {
        Self {
            log_target: format!("ImapPoll[{}]", name),
            config: config.clone(),
            worker: None,
        }
    }
}
impl MailAgent for ImapPollSource {
    fn join(&mut self) {
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("Thread exited with errors");
    }
}
impl MailSource for ImapPollSource {
    fn start(&mut self, channel: HubSourceChannel) {
        info!(target: &self.log_target, "Starting");
        trace!(target: &self.log_target, "Using Configuration:\n{:?}", self.config);

        let log_target = self.log_target.clone();
        let config = self.config.clone();

        self.worker = Some(thread::spawn(move || {
            let mut con = ImapConnection::new(config.server.clone(), config.port, config.auth);
            loop {
                info!(target: &log_target, "Polling for unread mails");
                con.iter_unseen_recursive(None, !config.keep)
                    .unwrap()
                    .for_each(|unseen_message| {
                        if let Ok((path, unseen_message)) = unseen_message {
                            debug!(target: &log_target, "Unread mail in {}", path);
                            channel.notify_new_mail(unseen_message);
                        }
                    });

                // sleep until next poll is due - interrupt if requested to stop
                if let Some(_) = channel.next_timeout(Duration::from_secs(config.interval)) {
                    // received stop request -> return, otherwise, do next poll round
                    info!(target: &log_target, "Stopping");
                    return;
                }
            }
        }));
    }
}
