use crate::hub::{HubDestinationChannel, MailAgent};

pub mod exec;
pub mod smtp;
pub mod testdst;

pub trait MailDestination: MailAgent {
    fn start(&mut self, channel: HubDestinationChannel);
}
