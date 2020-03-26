use super::config::{ConfigContainer, SourceConfig, DestinationConfig};
use crate::{
	destinations::smtp::SmtpDestination,
	sources::{
		imap_poll::ImapPollSource,
		imap_idle::ImapIdleSource,
		testsrc::TestSource
	}
};
use log::info;
use std::{
	sync::mpsc,
	collections::HashMap
};

#[derive(Clone, Debug)]
pub struct Mail {
	pub data: Vec<u8>
}
impl Mail {
	pub fn from_rfc822(body: Vec<u8>) -> Self {
		Self {
			data: body
		}
	}
}


pub trait MailAgent {
	fn start(&mut self);
	fn stop(&mut self);
}

pub trait MailSource : MailAgent {
}
pub trait MailDestination : MailAgent {
	fn enqueue(&mut self, mail: Mail);
}


pub enum MailHubMessage {
	Mail { srcname: String, mail: Mail },
	Shutdown
}
pub struct MailChannel {
	srcname: String,
	mail_sender: mpsc::Sender<MailHubMessage>
}
impl MailChannel {
	pub fn new(srcname: String, mail_sender: mpsc::Sender<MailHubMessage>) -> Self {
		Self {srcname, mail_sender}
	}
	pub fn notify(&self, mail: Mail) {
		self.mail_sender.send(MailHubMessage::Mail{
			srcname: self.srcname.clone(),
			mail
		}).unwrap();
	}
}


pub struct MailHubStopSender {
	stop_sender: mpsc::Sender<MailHubMessage>
}
impl MailHubStopSender {
	pub fn stop(&self) {
		self.stop_sender.send(MailHubMessage::Shutdown).unwrap();
	}
}

pub struct MailHub {
	destination_agents: HashMap<String, Box<dyn MailDestination>>,
	source_agents: HashMap<String, Box<dyn MailSource>>,
	mappings: HashMap<String, Vec<String>>,
	stop_sender: mpsc::Sender<MailHubMessage>,
	mail_receiver: mpsc::Receiver<MailHubMessage>
}
impl MailHub {
	pub fn from_config(config: &ConfigContainer) -> Self {
		let mut destination_agents = HashMap::new();
		let mut source_agents = HashMap::new();
		let mappings = config.mappings.clone();
		let (mail_sender, mail_receiver) = mpsc::channel();

		// Create destinations
		for (dstname, dstcfg) in &config.destinations {
			let destination_agent: Box<dyn MailDestination> = match dstcfg {
				DestinationConfig::Smtp(config) => Box::new(SmtpDestination::new(dstname.clone(), config))
			};
			destination_agents.insert(dstname.clone(), destination_agent);
		}

		// Create sources
		for (srcname, srccfg) in &config.sources {
			let mail_channel = MailChannel::new(srcname.clone(), mail_sender.clone());
			let source_agent: Box<dyn MailSource> = match srccfg {
				SourceConfig::Test => Box::new(TestSource::new(mail_channel)),
				SourceConfig::ImapPoll(config) => Box::new(ImapPollSource::new(srcname.clone(), config, mail_channel)),
				SourceConfig::ImapIdle(config) => Box::new(ImapIdleSource::new(srcname.clone(), config, mail_channel))
			};
			source_agents.insert(srcname.clone(), source_agent);
		}

		Self {destination_agents, source_agents, mappings, mail_receiver, stop_sender: mail_sender}
	}

	pub fn run(&mut self) {
		info!(target: "MailHub", "Starting.");
		for (dst_name, dst) in &mut self.destination_agents {
			info!(target: "MailHub", "Starting destination: {}", dst_name);
			dst.start();
		}
		for (src_name, src) in &mut self.source_agents {
			info!(target: "MailHub", "Starting source: {}", src_name);
			src.start();
		}

		info!(target: "MailHub", "Starting distribution loop");
		loop {
			let msg = self.mail_receiver.recv()
				.expect("Failed to read Mail from distribution channel");
			match msg {
				MailHubMessage::Shutdown => break,
				MailHubMessage::Mail { srcname, mail } => {
					info!(target: "MailHub", "Mail from source {}", srcname);
					if let Some(dstlist) = self.mappings.get(&srcname) {
						for dstname in dstlist {
							info!(target: "MailHub", "Distributing Mail {} => {}", srcname, dstname);
							let dst = self.destination_agents.get_mut(dstname).unwrap();
							dst.enqueue(mail.clone());
						}
					}
				}	
			}
		}
		info!(target: "MailHub", "Exited distribution loop");

		info!(target: "MailHub", "Shutting down");
		for (src_name, src) in &mut self.source_agents {
			info!(target: "MailHub", "Stopping source: {}", src_name);
			src.stop();
		}
		for (dst_name, dst) in &mut self.destination_agents {
			info!(target: "MailHub", "Stopping destination: {}", dst_name);
			dst.stop();
		}
	}

	pub fn get_stop_sender(&self) -> MailHubStopSender {
		MailHubStopSender {
			stop_sender: self.stop_sender.clone()
		}
	}
}