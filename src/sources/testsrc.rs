use crate::hub::{Mail, MailAgent, MailSource};
use lettre::builder::EmailBuilder;

pub struct TestSource {
    name: String,
}
impl TestSource {
    pub fn new(name: String) -> Self {
        Self { name }
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
            .html("<b>html/text</b>")
            .attachment(b"Test-Content", "test.txt", &mime::TEXT_PLAIN)
            .unwrap()
            .build()
            .unwrap();
        let mail_data = mail.message_to_string().unwrap().as_bytes().to_vec();

        channel.notify_new_mail(Mail::from_rfc822(self.name.clone(), mail_data));
    }
}
