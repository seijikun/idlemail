use crate::{
    config::TestDestinationConfig,
    hub::{DestinationMessage, HubDestinationChannel, MailAgent, MailDestination},
};
use log::{info, trace};
use std::thread;

pub struct TestDestination {
    log_target: String,
    config: TestDestinationConfig,
    worker: Option<thread::JoinHandle<()>>,
}
impl TestDestination {
    pub fn new(name: String, config: &TestDestinationConfig) -> Self {
        Self {
            log_target: format!("TestDst[{}]", name),
            config: config.clone(),
            worker: None,
        }
    }
}
impl MailAgent for TestDestination {
    fn join(&mut self) {}
}
impl MailDestination for TestDestination {
    fn start(&mut self, channel: HubDestinationChannel) {
        info!(target: &self.log_target, "Starting");
        trace!(target: &self.log_target, "Using Configuration:\n{:?}", self.config);

        let log_target = self.log_target.clone();
        let config = self.config.clone();
        self.worker = Some(thread::spawn(move || {
            let mut fails_remaining = config.fail_n_first;
            while let Ok(msg) = channel.next() {
                match msg {
                    DestinationMessage::Shutdown => return,
                    DestinationMessage::Mail { mail } => {
                        if fails_remaining > 0 {
                            info!(target: &log_target, "Got Mail: Simulating send failure.");
                            fails_remaining -= 1;
                            channel.notify_failed_send(mail);
                        } else {
                            info!(target: &log_target, "Got Mail: Simulating success");
                        }
                    }
                }
            }
        }));
    }
}
