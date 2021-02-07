use crate::hub::{Mail, MailAgent, MailSource};
use lettre::{
    message::{header, Mailbox, MultiPart, SinglePart},
    Message,
};

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
        let body_html = SinglePart::builder()
            .header(header::ContentType(
                "text/html; charset=utf8".parse().unwrap(),
            ))
            .body("<b>text/html</b>".to_owned());
        let body_text = SinglePart::builder()
            .header(header::ContentType(
                "text/plain; charset=utf8".parse().unwrap(),
            ))
            .body("text/plain".to_owned());
        let body = MultiPart::alternative()
            .singlepart(body_html)
            .singlepart(body_text);

        let mail = Message::builder()
            .from(Mailbox::new(None, "sender@example.org".parse().unwrap()))
            .to(Mailbox::new(None, "receiver@example.or".parse().unwrap()))
            .subject("Test Email")
            .date_now()
            .multipart(body)
            .unwrap();

        channel.notify_new_mail(Mail::from_rfc822(self.name.clone(), mail.formatted()));
    }
}
