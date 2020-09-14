use super::common::{ImapConnection, MailPath};
use crate::{
    config::ImapIdleSourceConfig,
    hub::{HubSourceChannel, Mail, MailAgent, MailSource},
};
use async_std::task;
use futures::{future::FutureExt, pin_mut, select};
use log::{debug, error, info, trace, warn};
use std::{thread, time::Duration};

pub struct ImapIdleSource {
    name: String,
    log_target: String,
    config: ImapIdleSourceConfig,
    worker: Option<thread::JoinHandle<()>>,
}
impl ImapIdleSource {
    pub fn new(name: String, config: &ImapIdleSourceConfig) -> Self {
        Self {
            log_target: format!("ImapIdle[{}]", name),
            name,
            config: config.clone(),
            worker: None,
        }
    }
}
impl MailAgent for ImapIdleSource {
    fn join(&mut self) {
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("Thread exited with errors");
    }
}
impl MailSource for ImapIdleSource {
    fn start(&mut self, channel: HubSourceChannel) {
        info!(target: &self.log_target, "Starting");
        trace!(target: &self.log_target, "Using Configuration:\n{:?}", self.config);

        let name = self.name.clone();
        let log_target = self.log_target.clone();
        let config = self.config.clone();

        self.worker = Some(thread::spawn(move || {
            let mut con =
                ImapConnection::new(config.server.clone(), config.port, config.auth.clone());

            let stop_future = channel.next().fuse();
            pin_mut!(stop_future);

            loop {
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

                loop { // inner loop used only if something fails while entering IDLE state and we need to retry
                    debug!(
                        target: &log_target,
                        "Entering IMAP IDLE to wait for server notification"
                    );
                    match con.run(|sess| task::block_on(sess.select(config.path.clone()))) {
                        Ok(_) => {}
                        Err(e) => {
                            error!(target: &log_target, "Failed to enter IMAP IDLE state:\n{}", e.backtrace());
                            // connection-lost errors should be handled by the connection, so this could
                            // be an authentication error, or a temporary unavailable server. Wait a bit and retry
                            thread::sleep(Duration::from_secs(5));
                            continue;
                        }
                    }
                    let mut idle_handle = match con.idle() {
                        Ok(idle_handle) => idle_handle,
                        Err(e) => {
                            error!(target: &log_target, "Failed to enter IMAP IDLE state:\n{}", e.backtrace());
                            thread::sleep(Duration::from_secs(5));
                            continue;
                        }
                    };
                    // dropping the StopSource interrupts idle, the variable thus needs a name.
                    let (idle_future, _stopsrc) =
                        idle_handle.wait_with_timeout(Duration::from_secs(config.renewinterval));
    
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
                    if should_exit {
                        info!(target: &log_target, "Stopping");
                        return;
                    }
                    debug!(target: &log_target, "IDLE interrupted");
                    break; // no error -> go to outer loop to fetch mails and return to the IDLE state
                }
            }
        }));
    }
}
