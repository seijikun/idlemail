use crate::hub::{HubSourceChannel, MailAgent};

mod common;
pub mod imap_idle;
pub mod imap_poll;
pub mod testsrc;

pub trait MailSource: MailAgent {
    fn start(&mut self, channel: HubSourceChannel);
}
