use crate::hub::{MailAgent, MailSource, MailChannel, Mail};
use lettre::builder::EmailBuilder;

pub struct TestSource {
	mail_channel: MailChannel
}
impl TestSource {
	pub fn new(mail_channel: MailChannel) -> Self {
		Self {mail_channel}
	}
}
impl MailAgent for TestSource {
    fn start(&mut self) {
		let mail = EmailBuilder::new()
			.from("sender@example.org")
			.to("receiver@example.or")
			.subject("Test Email")
			.date(&time::OffsetDateTime::now())
			.text("plain/Text")
			.html("html/text")
			.attachment(
				"Test-Content".as_bytes(),
				"test.txt",
				&mime::TEXT_PLAIN
			).unwrap()
			.build()
			.unwrap();
		let mail_data = mail.message_to_string().unwrap().as_bytes().to_vec();

		self.mail_channel.notify(Mail::from_rfc822(mail_data));
	}
    fn stop(&mut self) {}
}
impl MailSource for TestSource {}