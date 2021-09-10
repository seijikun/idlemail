use std::{sync::mpsc, thread, time::Duration};

use crate::{
    config::TestSourceConfig,
    hub::{Mail, MailAgent, MailSource},
};
use lettre::{
    message::{header, Mailbox, MultiPart, SinglePart},
    Message,
};

pub struct TestSource {
    name: String,
    config: TestSourceConfig,
    worker: Option<(mpsc::Sender<()>, thread::JoinHandle<()>)>,
}
impl TestSource {
    pub fn new(name: String, config: &TestSourceConfig) -> Self {
        Self {
            name,
            config: config.clone(),
            worker: None,
        }
    }

    fn send_testmail(name: String, channel: &crate::hub::HubSourceChannel) {
        let body_html = SinglePart::builder()
            .header(header::ContentType::parse("text/html; charset=utf8").unwrap())
            .body("<b>text/html</b>".to_owned());
        let body_text = SinglePart::builder()
            .header(header::ContentType::parse("text/plain; charset=utf8").unwrap())
            .body("text/plain".to_owned());
        let body = MultiPart::alternative()
            .singlepart(body_html)
            .singlepart(body_text);

        let testmail = Message::builder()
            .from(Mailbox::new(None, "sender@example.org".parse().unwrap()))
            .to(Mailbox::new(None, "receiver@example.or".parse().unwrap()))
            .subject("Test Email")
            .date_now()
            .multipart(body)
            .unwrap();

        channel.notify_new_mail(Mail::from_rfc822(name, testmail.formatted()));
    }
}
impl MailAgent for TestSource {
    fn join(&mut self) {
        if let Some((_, handle)) = self.worker.take() {
            let _ = handle.join();
        }
    }
}
impl MailSource for TestSource {
    fn start(&mut self, channel: crate::hub::HubSourceChannel) {
        let name = self.name.clone();
        let config = self.config.clone();
        let (stop_tx, stop_rx) = mpsc::channel();

        self.worker = Some((
            stop_tx,
            thread::spawn(move || {
                thread::sleep(Duration::from_secs(config.delay));
                loop {
                    TestSource::send_testmail(name.clone(), &channel);
                    match stop_rx.recv_timeout(Duration::from_secs(config.interval)) {
                        // timeout hit, send new produce and schedule new test-mail
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        // we were asked to exit
                        _ => break,
                    }
                }
            }),
        ));
    }
}
