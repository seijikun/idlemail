use super::config::{ConfigContainer, DestinationConfig, SourceConfig};
use crate::{
    config::RetryAgentConfig,
    destinations::{
        exec::ExecDestination, smtp::SmtpDestination, testdst::TestDestination, MailDestination,
    },
    retryagents::{filesystem::FilesystemRetryAgent, memory::MemoryRetryAgent, MailRetryAgent},
    sources::{
        imap_idle::ImapIdleSource, imap_poll::ImapPollSource, testsrc::TestSource, MailSource,
    },
};
use async_std::{channel as async_mpsc, future::timeout as await_timeout, task};
use log::{info, warn};
use mpsc::RecvError;
use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    sync::mpsc,
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Mail {
    pub from_src: String,
    pub data: Vec<u8>,
    pub hash: String,
}
impl Mail {
    pub fn from_rfc822(srcname: String, body: Vec<u8>) -> Self {
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        Self {
            from_src: srcname,
            data: body,
            hash: hasher.finish().to_string(),
        }
    }
}

pub enum HubMessage {
    NewMail {
        srcname: String,
        mail: Mail,
    },
    RetryMail {
        dstname: String,
        mail: Mail,
    },
    SendingMailFailed {
        dstname: String,
        mail: Mail,
    },
    Shutdown,
    /// Message sent by the RetryAgent to confirm successfull suspension
    RetryAgentSuspended,
}
pub struct HubChannel {
    sender: mpsc::Sender<HubMessage>,
    recv: mpsc::Receiver<HubMessage>,
    destinations: HashMap<String, mpsc::Sender<DestinationMessage>>,
    sources: HashMap<String, async_mpsc::Sender<SourceMessage>>,
    retryagent_sender: Option<mpsc::Sender<RetryAgentMessage>>,
    retryagent_recv: Option<mpsc::Receiver<RetryAgentMessage>>,
}
impl HubChannel {
    pub fn new() -> Self {
        let (main_sender, main_recv) = mpsc::channel();
        let (retryagent_sender, retryagent_recv) = mpsc::channel();
        Self {
            sender: main_sender,
            recv: main_recv,
            destinations: HashMap::new(),
            sources: HashMap::new(),
            retryagent_sender: Some(retryagent_sender),
            retryagent_recv: Some(retryagent_recv),
        }
    }

    pub fn next(&self) -> HubMessage {
        self.recv.recv().unwrap()
    }
    pub fn try_next(&self) -> Option<HubMessage> {
        self.recv.try_recv().ok()
    }

    pub fn queue_mail_for_sending(&self, dstname: &str, mail: Mail) -> Result<(), ()> {
        let dst_comm = self.destinations.get(dstname).ok_or(())?;
        dst_comm
            .send(DestinationMessage::Mail { mail })
            .map_err(|_| ())
    }

    pub fn queue_mail_for_retry(&self, dstname: String, mail: Mail) {
        if self
            .retryagent_sender
            .as_ref()
            .unwrap()
            .send(RetryAgentMessage::QueueMail { dstname, mail })
            .is_err()
        {
            warn!(target: "HubChannel", "Failed to queue mail for retransmission. Either no RetryAgent configured, or a bug.");
        }
    }

    pub fn shutdown_sources(&mut self) {
        info!(target: "HubChannel", "Signaling shutdown to sources");
        self.sources.clear();
    }
    pub fn shutdown_destinations(&mut self) {
        info!(target: "HubChannel", "Signaling shutdown to destinations");
        self.destinations.clear();
    }
    pub fn suspend_retryagent(&mut self) {
        info!(target: "HubChannel", "Suspending retryagent");
        let _ = self
            .retryagent_sender
            .as_ref()
            .unwrap()
            .send(RetryAgentMessage::Suspend);
    }
    pub fn shutdown_retryagent(&mut self) {
        info!(target: "HubChannel", "Signaling shutdown to retryagent");
        drop(self.retryagent_sender.take());
    }

    pub fn get_stop_channel(&self) -> HubStopSender {
        HubStopSender {
            sender: self.sender.clone(),
        }
    }
    pub fn get_destination_channel(&mut self, name: String) -> HubDestinationChannel {
        let (dst_send, dst_recv) = mpsc::channel();
        self.destinations.insert(name.clone(), dst_send);
        HubDestinationChannel {
            name,
            sender: self.sender.clone(),
            recv: dst_recv,
        }
    }
    pub fn get_source_channel(&mut self, name: String) -> HubSourceChannel {
        let (src_send, src_recv) = async_mpsc::bounded(1);
        self.sources.insert(name.clone(), src_send);
        HubSourceChannel {
            name,
            sender: self.sender.clone(),
            recv: src_recv,
        }
    }
    pub fn get_retryagent_channel(&mut self) -> HubRetryAgentChannel {
        HubRetryAgentChannel {
            sender: self.sender.clone(),
            recv: self.retryagent_recv.take().unwrap(),
        }
    }
}

pub struct HubStopSender {
    pub(crate) sender: mpsc::Sender<HubMessage>,
}
impl HubStopSender {
    pub fn stop(&self) {
        self.sender.send(HubMessage::Shutdown).unwrap();
    }
}

pub enum DestinationMessage {
    Mail { mail: Mail },
}
pub struct HubDestinationChannel {
    pub(crate) name: String,
    pub(crate) sender: mpsc::Sender<HubMessage>,
    pub(crate) recv: mpsc::Receiver<DestinationMessage>,
}
impl HubDestinationChannel {
    pub fn next(&self) -> Result<DestinationMessage, RecvError> {
        self.recv.recv()
    }

    pub fn notify_failed_send(&self, mail: Mail) {
        self.sender
            .send(HubMessage::SendingMailFailed {
                dstname: self.name.clone(),
                mail,
            })
            .unwrap();
    }
}

pub enum SourceMessage {}
pub struct HubSourceChannel {
    pub(crate) name: String,
    pub(crate) sender: mpsc::Sender<HubMessage>,
    pub(crate) recv: async_mpsc::Receiver<SourceMessage>,
}
impl HubSourceChannel {
    pub async fn next(&self) -> Option<SourceMessage> {
        self.recv.recv().await.ok()
    }
    pub fn next_timeout(&self, timeout: Duration) -> Result<SourceMessage, mpsc::RecvTimeoutError> {
        // match async interface to synchrnous std interface for consistency
        match task::block_on(await_timeout(timeout, self.recv.recv())) {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(_)) => Err(mpsc::RecvTimeoutError::Disconnected),
            Err(_) => Err(mpsc::RecvTimeoutError::Timeout),
        }
    }
    pub fn notify_new_mail(&self, mail: Mail) {
        self.sender
            .send(HubMessage::NewMail {
                srcname: self.name.clone(),
                mail,
            })
            .unwrap();
    }
}

pub enum RetryAgentMessage {
    QueueMail {
        dstname: String,
        mail: Mail,
    },
    /// Sending this message to a running RetryAgent suspends its re-submission attempts.
    /// This means, that the RetryAgent will still receive and handle incomming messages
    /// to be re-submitted, but no actual resubmission will be sent to the hub.
    /// This is used during shutdown, so persistent RetryAgents can store the incomming
    /// messages for resubmission, but does not attempt actual resubmission so the destinations
    /// can shut down.
    Suspend,
}
pub struct HubRetryAgentChannel {
    sender: mpsc::Sender<HubMessage>,
    recv: mpsc::Receiver<RetryAgentMessage>,
}
impl HubRetryAgentChannel {
    pub fn next_timeout(
        &self,
        timeout: Duration,
    ) -> Result<RetryAgentMessage, mpsc::RecvTimeoutError> {
        self.recv.recv_timeout(timeout)
    }
    pub fn notify_retry_mail(&self, dstname: String, mail: Mail) {
        self.sender
            .send(HubMessage::RetryMail { dstname, mail })
            .unwrap();
    }
    pub fn confirm_suspension(&self) {
        self.sender.send(HubMessage::RetryAgentSuspended).unwrap();
    }
}

pub trait MailAgent {
    fn join(&mut self);
}

pub struct MailHub {
    destination_agents: HashMap<String, Box<dyn MailDestination>>,
    source_agents: HashMap<String, Box<dyn MailSource>>,
    retryagent: Option<Box<dyn MailRetryAgent>>,
    mappings: HashMap<String, Vec<String>>,
    hubchannel: HubChannel,
}
impl MailHub {
    pub fn from_config(config: &ConfigContainer) -> Self {
        let mut destination_agents = HashMap::new();
        let mut source_agents = HashMap::new();
        let hubchannel = HubChannel::new();

        // Create destinations
        for (dstname, dstcfg) in &config.destinations {
            let destination_agent: Box<dyn MailDestination> = match dstcfg {
                DestinationConfig::Test(config) => {
                    Box::new(TestDestination::new(dstname.clone(), config))
                }
                DestinationConfig::Smtp(config) => {
                    Box::new(SmtpDestination::new(dstname.clone(), config))
                }
                DestinationConfig::Exec(config) => {
                    Box::new(ExecDestination::new(dstname.clone(), config))
                }
            };
            destination_agents.insert(dstname.clone(), destination_agent);
        }

        // Create sources
        for (srcname, srccfg) in &config.sources {
            let source_agent: Box<dyn MailSource> = match srccfg {
                SourceConfig::Test(config) => Box::new(TestSource::new(srcname.clone(), config)),
                SourceConfig::ImapPoll(config) => {
                    Box::new(ImapPollSource::new(srcname.clone(), config))
                }
                SourceConfig::ImapIdle(config) => {
                    Box::new(ImapIdleSource::new(srcname.clone(), config))
                }
            };
            source_agents.insert(srcname.clone(), source_agent);
        }

        let retryagent = config.retryagent.as_ref().map(|c| {
            let retryagent: Box<dyn MailRetryAgent> = match c {
                RetryAgentConfig::Memory(config) => Box::new(MemoryRetryAgent::new(config)),
                RetryAgentConfig::Filesystem(config) => Box::new(FilesystemRetryAgent::new(config)),
            };
            retryagent
        });

        Self {
            destination_agents,
            source_agents,
            retryagent,
            mappings: config.mappings.clone(),
            hubchannel,
        }
    }

    fn handle_message(&self, msg: HubMessage) -> bool {
        match msg {
            HubMessage::Shutdown => {
                return true;
            }
            HubMessage::RetryAgentSuspended => {
                return true;
            }
            HubMessage::NewMail { srcname, mail } => {
                info!(target: "MailHub", "Mail from source {}", srcname);
                if let Some(dstlist) = self.mappings.get(&srcname) {
                    for dstname in dstlist {
                        info!(target: "MailHub", "Distributing Mail {} => {}", srcname, dstname);
                        self.hubchannel
                            .queue_mail_for_sending(dstname, mail.clone())
                            .expect("Failed to distribute mail");
                    }
                }
            }
            HubMessage::SendingMailFailed { dstname, mail } => {
                info!(target: "MailHub", "Queueing failed mail for retransmission");
                self.hubchannel.queue_mail_for_retry(dstname, mail);
            }
            HubMessage::RetryMail { dstname, mail } => {
                info!(target: "MailHub", "Distributing Mail [retry] => {}", dstname);
                self.hubchannel
                    .queue_mail_for_sending(&dstname, mail)
                    .expect("Failed to distribute mail");
            }
        }
        false
    }

    pub fn run(&mut self) {
        info!(target: "MailHub", "Starting.");
        for (dst_name, dst) in &mut self.destination_agents {
            info!(target: "MailHub", "Starting destination: {}", dst_name);
            let comm = self.hubchannel.get_destination_channel(dst_name.clone());
            dst.start(comm);
        }
        if let Some(ref mut retryagent) = self.retryagent {
            info!(target: "MailHub", "Starting retryagent");
            let comm = self.hubchannel.get_retryagent_channel();
            retryagent.start(comm);
        }
        for (src_name, src) in &mut self.source_agents {
            info!(target: "MailHub", "Starting source: {}", src_name);
            let comm = self.hubchannel.get_source_channel(src_name.clone());
            src.start(comm);
        }

        info!(target: "MailHub", "Starting distribution loop");
        loop {
            let msg = self.hubchannel.next();
            if self.handle_message(msg) {
                break;
            }
        }
        info!(target: "MailHub", "Exited distribution loop");
        info!(target: "MailHub", "Shutting down");

        // Shutdown procedure
        // ####################

        // First, we suspend the sources to stop the stream of new mails incomming
        self.hubchannel.shutdown_sources();
        for (src_name, src) in &mut self.source_agents {
            src.join();
            info!(target: "MailHub", "Source: {} stopped", src_name);
        }

        // Then, we suspend the retry-agent, so it does still take incomming mails to-be
        // retried, but it does not actually schedule them (send them to the hub).
        self.hubchannel.suspend_retryagent();

        // Wait for retryagent to confirm suspension and handle all messages until then
        // (there might still be some resubmissions sent to destinations here)
        loop {
            let msg = self.hubchannel.next();
            if self.handle_message(msg) {
                break;
            }
        }

        // The destinations can now finish the mails they have queued (which might schedule
        // new mails in the retryagents), but no new mails are queued into destinations to send.
        self.hubchannel.shutdown_destinations();
        for (dst_name, dst) in &mut self.destination_agents {
            dst.join();
            info!(target: "MailHub", "Destination: {} stopped", dst_name);
        }

        // Handle all resubmission HubMessages that have accumulated before we tell the retryagent to shut down
        // If the retryagent is a persistent one, it can store the new mails to be retried, it won't
        // try to schedule them, since it's suspended.
        while let Some(msg) = self.hubchannel.try_next() {
            self.handle_message(msg);
        }

        // Last, the retryagent is shutdown.
        self.hubchannel.shutdown_retryagent();
        if let Some(retryagent) = &mut self.retryagent {
            retryagent.join();
            info!(target: "MailHub", "Retryagent stopped");
        }
    }

    pub fn get_stop_sender(&self) -> HubStopSender {
        self.hubchannel.get_stop_channel()
    }
}
