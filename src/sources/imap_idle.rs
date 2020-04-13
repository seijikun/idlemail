use super::common::ImapConnection;
use crate::{
    config::ImapIdleSourceConfig,
    hub::{HubSourceChannel, MailAgent, MailSource},
};
use async_std::task;
use futures::{future::FutureExt, pin_mut, select};
use log::{debug, info, trace};
use std::{thread, time::Duration};

pub struct ImapIdleSource {
    log_target: String,
    config: ImapIdleSourceConfig,
    worker: Option<thread::JoinHandle<()>>,
}
impl ImapIdleSource {
    pub fn new(name: String, config: &ImapIdleSourceConfig) -> Self {
        Self {
            log_target: format!("ImapIdle[{}]", name),
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

        let log_target = self.log_target.clone();
        let config = self.config.clone();

        self.worker = Some(thread::spawn(move || {
            let mut con =
                ImapConnection::new(config.server.clone(), config.port, config.auth.clone());

            let stop_future = channel.next().fuse();
            pin_mut!(stop_future);

            loop {
                con.iter_unseen_recursive(Some(&config.path), !config.keep)
                    .unwrap()
                    .for_each(|unseen_message| {
                        if let Ok((path, unseen_message)) = unseen_message {
                            debug!(target: &log_target, "Unread mail in {}", path);
                            channel.notify_new_mail(unseen_message);
                        }
                    });

                info!(
                    target: &log_target,
                    "Entering IMAP IDLE to wait for server notification"
                );
                con.run(|sess| task::block_on(sess.select(config.path.clone())))
                    .unwrap();
                let mut idle_handle = con.idle().unwrap();
                // Dropping the StopSource interrupts idle, the variable thus needs a name.
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
            }
        }));
    }
}
