use crate::{
    config::ExecDestinationConfig,
    hub::{DestinationMessage, HubDestinationChannel, MailAgent, MailDestination},
};
use log::{debug, error, info, log_enabled, trace, Level as log_level};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    thread,
};

pub struct ExecDestination {
    name: String,
    log_target: String,
    config: ExecDestinationConfig,
    worker: Option<thread::JoinHandle<()>>,
}
impl ExecDestination {
    pub fn new(name: String, config: &ExecDestinationConfig) -> Self {
        Self {
            log_target: format!("ExecDst[{}]", name),
            name,
            config: config.clone(),
            worker: None,
        }
    }
}
impl MailAgent for ExecDestination {
    fn join(&mut self) {}
}
impl MailDestination for ExecDestination {
    fn start(&mut self, channel: HubDestinationChannel) {
        info!(target: &self.log_target, "Starting");
        trace!(target: &self.log_target, "Using Configuration:\n{:?}", self.config);

        let name = self.name.clone();
        let log_target = self.log_target.clone();
        let config = self.config.clone();
        self.worker = Some(thread::spawn(move || {
            while let Ok(DestinationMessage::Mail { mail }) = channel.next() {
                // spawn the process with the apropriate configuration (args, env, ..)
                let mut exec_config = Command::new(&config.executable);
                exec_config.stdin(Stdio::piped()).stdout(Stdio::piped());
                if let Some(arguments) = config.arguments.as_ref() {
                    exec_config.args(arguments);
                }
                if let Some(environment) = config.environment.as_ref() {
                    exec_config.envs(environment);
                }
                exec_config.env("IDLEMAIL_DESTINATION", &name);
                exec_config.env("IDLEMAIL_SOURCE", &mail.from_src);

                match exec_config.spawn() {
                    Ok(ref mut child) => {
                        match child.stdin.as_mut().map(|stdin| stdin.write(&mail.data)) {
                            Some(Ok(_)) => {
                                // we successfully opened stdin, and piped the mail to the child
                                // wait for child to exit
                                let child_result = child.wait();
                                if log_enabled!(log_level::Debug) {
                                    // if debug log is enabled, print child output
                                    // we do this manually to ensure, that child-output is one block in the log
                                    // child messages randomly mixed in would be ugly
                                    if let Some(stdout) = child.stdout.as_mut() {
                                        let mut child_log = String::new();
                                        if stdout.read_to_string(&mut child_log).is_ok() {
                                            debug!(
                                                target: &format!("{}[Child]", log_target),
                                                "{}", child_log
                                            );
                                        }
                                    }
                                }
                                // handle child exit status
                                match child_result {
                                    Ok(res) => {
                                        debug!(
                                            target: &log_target,
                                            "Successfully sent mail to child"
                                        );
                                        if res.success() {
                                            info!(
                                                target: &log_target,
                                                "Child exited with: {}",
                                                res.code().unwrap_or(0)
                                            );
                                            continue;
                                        } else {
                                            error!(
                                                target: &log_target,
                                                "Child exited with: {}",
                                                res.code().unwrap_or(-1)
                                            );
                                        }
                                    }
                                    Err(err) => error!(
                                        target: &log_target,
                                        "Child exited with error: {}", err
                                    ),
                                }
                            }
                            Some(Err(err)) => {
                                // piping stdin to the child went wrong
                                error!(
                                    target: &log_target,
                                    "Error while piping mail to spawned process: {}", err
                                )
                            }
                            None => {
                                error!(target: &log_target, "Failed to open stdin of child process")
                            }
                        }
                    }
                    Err(err) => {
                        error!(
                            target: &log_target,
                            "Error while spawning configured executable: {}", err
                        );
                    }
                }
                // if we made it here (we continue on success), something went wrong
                channel.notify_failed_send(mail);
            }
            info!(target: &log_target, "Stopping");
        }));
    }
}
