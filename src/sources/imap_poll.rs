use super::common::{ImapConnection, MailPath};
use crate::{
    config::ImapPollSourceConfig,
    hub::{HubSourceChannel, Mail, MailAgent, MailSource},
};
use async_std::task;
use log::{debug, error, info, trace, warn};
use std::{thread, time::Duration};

pub struct ImapPollSource {
    name: String,
    log_target: String,
    config: ImapPollSourceConfig,
    worker: Option<thread::JoinHandle<()>>,
}
impl ImapPollSource {
    pub fn new(name: String, config: &ImapPollSourceConfig) -> Self {
        Self {
            log_target: format!("ImapPoll[{}]", name),
            name,
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

        let name = self.name.clone();
        let log_target = self.log_target.clone();
        let config = self.config.clone();

        self.worker = Some(thread::spawn(move || {
            let con = ImapConnection::new(config.server.clone(), config.port, config.auth.clone());
            loop {
                debug!(target: &log_target, "Polling for unread mails");
                match con.iter_mailboxes_recursive(None) {
                    Ok(mailboxes) => {
                        mailboxes.for_each(|mailbox| {
                            let mut unread_mails = Vec::new();
                            con.iter_unseen(&mailbox)
                                .unwrap()
                                .for_each(|unseen_message| {
                                    if let Ok((message_id, unseen_message)) = unseen_message {
                                        unread_mails.push(message_id);
                                        debug!(
                                            target: &log_target,
                                            "Unread mail in {}",
                                            mailbox.path()
                                        );
                                        channel.notify_new_mail(Mail::from_rfc822(
                                            name.clone(),
                                            unseen_message,
                                        ));
                                    }
                                });
                            if !config.keep {
                                if let Err(e) = task::block_on(con.delete_mails(&unread_mails)) {
                                    warn!(
                                        target: &log_target,
                                        "Failed to deleted messages from mailbox\n{}", e
                                    );
                                }
                            }
                        });
                    }
                    Err(e) => {
                        error!(
                            target: &log_target,
                            "Failed to get recursive list of mailboxes to iterate\n{}",
                            e.backtrace()
                        );
                    }
                }

                // sleep until next poll is due - interrupt if requested to stop
                if channel
                    .next_timeout(Duration::from_secs(config.interval))
                    .is_some()
                {
                    // received stop request -> return, otherwise, do next poll round
                    info!(target: &log_target, "Stopping");
                    return;
                }
            }
        }));
    }
}
