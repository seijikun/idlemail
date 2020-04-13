use crate::hub::{Mail, MailAgent, MailSource};
use lettre::builder::EmailBuilder;

pub struct TestSource {}
impl TestSource {
    pub fn new() -> Self {
        Self {}
    }
}
impl MailAgent for TestSource {
    fn join(&mut self) {}
}
impl MailSource for TestSource {
    fn start(&mut self, channel: crate::hub::HubSourceChannel) {
        let mail = EmailBuilder::new()
            .from("sender@example.org")
            .to("receiver@example.or")
            .subject("Test Email")
            .date(&time::OffsetDateTime::now())
            .text("plain/Text")
            .html("html/text")
            .attachment(b"Test-Content", "test.txt", &mime::TEXT_PLAIN)
            .unwrap()
            .build()
            .unwrap();
        let mail_data = mail.message_to_string().unwrap().as_bytes().to_vec();

        channel.notify_new_mail(Mail::from_rfc822(mail_data));
    }
}
