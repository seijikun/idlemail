use crate::{
    config::FilesystemRetryAgentConfig,
    hub::{Mail, MailAgent, MailRetryAgent, RetryAgentMessage},
};
use anyhow::Result;
use log::{debug, error, info, warn};
use serde_derive::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs, thread,
    time::{Duration, SystemTime},
};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
struct QueuedRetryMailModel {
    pub due_time: SystemTime,
    pub dstname: String,
    pub mail_from_src: String,
    pub mail_data: Vec<u8>,
}
impl From<&QueuedRetryMail> for QueuedRetryMailModel {
    fn from(retry_mail: &QueuedRetryMail) -> Self {
        Self {
            due_time: retry_mail.due_time,
            dstname: retry_mail.dstname.clone(),
            mail_from_src: retry_mail.mail.from_src.clone(),
            mail_data: retry_mail.mail.data.clone(),
        }
    }
}

struct QueuedRetryMail {
    pub due_time: SystemTime,
    pub dstname: String,
    pub mail: Mail,
    pub file_path: String,
}

pub struct FilesystemRetryAgent {
    log_target: String,
    config: FilesystemRetryAgentConfig,
    worker: Option<thread::JoinHandle<()>>,
}
impl FilesystemRetryAgent {
    pub fn new(config: &FilesystemRetryAgentConfig) -> Self {
        Self {
            log_target: "RetryAgent[Filesystem]".to_string(),
            config: config.clone(),
            worker: None,
        }
    }

    fn load_from_fs(&self) -> Result<Vec<QueuedRetryMail>> {
        let mail_files: Vec<QueuedRetryMail> = fs::read_dir(&self.config.path)?
			.filter_map(|file| {
				let file_path = file.ok()?.path();
				let file_path_str = file_path.to_str()?.to_owned();
				if !file_path_str.ends_with(".json") {
					return None;
				}
				let file_reader = match fs::File::open(file_path) {
					Ok(file_reader) => file_reader,
					Err(e) => {
						error!(target: &self.log_target, "Failed to open retry-file: {}\n{}", file_path_str, e);
						return None;
					}
				};
				let retry_mail: QueuedRetryMailModel = serde_json::from_reader(file_reader).ok()?;
				info!(target: &self.log_target, "Successfully parsed retry-file: {}", file_path_str);
				Some(QueuedRetryMail {
					due_time: retry_mail.due_time,
					dstname: retry_mail.dstname,
					mail: Mail::from_rfc822(retry_mail.mail_from_src, retry_mail.mail_data),
					file_path: file_path_str
				})
			})
			.collect();
        Ok(mail_files)
    }
}
impl MailAgent for FilesystemRetryAgent {
    fn join(&mut self) {
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("Thread exited with errors");
    }
}
impl MailRetryAgent for FilesystemRetryAgent {
    fn start(&mut self, channel: crate::hub::HubRetryAgentChannel) {
        let config = self.config.clone();
        let log_target = self.log_target.clone();
        info!(
            target: &log_target,
            "Loading messages from folder: {}", config.path
        );
        let mut restored_mails = match self.load_from_fs() {
            Ok(restored_mails) => restored_mails,
            Err(e) => {
                error!(
                    target: &log_target,
                    "Failed to load retry-mails from filesystem:\n{}", e
                );
                Vec::new()
            }
        };

        self.worker = Some(thread::spawn(move || {
            // We depend on the VecDequeue to be sorted by ascending due-time
            restored_mails.sort_by_key(|rm| rm.due_time);
            let mut queue: VecDeque<QueuedRetryMail> = VecDeque::from(restored_mails);

            loop {
                if let Some(msg) = channel.next_timeout(Duration::from_secs(1)) {
                    // got a new message, handle it
                    match msg {
                        RetryAgentMessage::Shutdown => {
                            info!(target: &log_target, "Stopping");
                            return;
                        }
                        RetryAgentMessage::QueueMail { dstname, mail } => {
                            let retransmission_timepoint =
                                SystemTime::now() + Duration::from_secs(config.delay);
                            info!(
                                target: &log_target,
                                "Queueing mail {} for retransmission in {}s",
                                mail.hash,
                                config.delay
                            );

                            // construct QueuedRetryMail structure, and attempt to find a non-taken filename
                            // for it in our designated filesystem path.
                            let mut retry_mail = QueuedRetryMail {
                                due_time: retransmission_timepoint,
                                dstname,
                                mail,
                                file_path: "".to_owned(),
                            };
                            for i in 0..10 {
                                // try 10 append-indices against hash-collision
                                let file_name = format!(
                                    "{}/{}_to_{}-{}.json",
                                    config.path, &retry_mail.mail.hash, retry_mail.dstname, i
                                );
                                if let Ok(retry_file) = fs::File::create(&file_name) {
                                    retry_mail.file_path = file_name.clone();
                                    match serde_json::to_writer(
                                        retry_file,
                                        &QueuedRetryMailModel::from(&retry_mail),
                                    ) {
                                        Ok(_) => {
                                            debug!(
                                                target: &log_target,
                                                "Stored retry-mail in: {}", retry_mail.file_path
                                            );
                                            queue.push_back(retry_mail);
                                            break;
                                        }
                                        Err(e) => {
                                            error!(
                                                target: &log_target,
                                                "Failed to create retry-mail file: {}\n{}",
                                                retry_mail.file_path,
                                                e
                                            );
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // see if any of the queued mails is due
                let now = SystemTime::now();
                while !queue.is_empty() {
                    if queue.get(0).unwrap().due_time < now {
                        let mail = queue.pop_front().unwrap();
                        info!(
                            target: &log_target,
                            "Mail {} due for retransmission. Queueing.", mail.mail.hash
                        );
                        channel.notify_retry_mail(mail.dstname, mail.mail);
                        if let Err(e) = fs::remove_file(&mail.file_path) {
                            warn!(
                                target: &log_target,
                                "Failed to delete retry-mail file: {}\n{}", mail.file_path, e
                            );
                        } else {
                            debug!(
                                target: &log_target,
                                "Deleted retry-mail file: {}", mail.file_path
                            );
                        }
                    } else {
                        // The mails are stored in the order in which they were queued.
                        // If the first isn't due, neither is every mail behind that.
                        break;
                    }
                }
            }
        }));
    }
}
