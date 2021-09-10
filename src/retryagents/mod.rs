use crate::hub::{HubRetryAgentChannel, MailAgent};

pub mod filesystem;
pub mod memory;

pub trait MailRetryAgent: MailAgent {
    fn start(&mut self, channel: HubRetryAgentChannel);
}
