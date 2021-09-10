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
    fn join(&mut self) {
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("Thread exited with errors");
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Mail;
    use lettre::{
        message::{header, Mailbox, MultiPart, SinglePart},
        Message,
    };
    use std::{
        collections::HashMap,
        fs::{File, Permissions},
        os::unix::prelude::PermissionsExt,
        path::PathBuf,
        sync::mpsc,
    };
    use tempfile::TempDir;
    use test_case::test_case;

    macro_rules! define_envargs_method(
        { $name:ident : $($key:expr => $value:expr),+ } => {
            fn $name() -> HashMap<String, String> {
                let mut m = ::std::collections::HashMap::new();
                $(
                    m.insert($key, $value);
                )+
                m
            }
         };
    );

    fn create_testmail(srcname: String) -> Mail {
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
        Mail::from_rfc822(srcname, testmail.formatted())
    }

    fn prepare_validation_script(src: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let executable_path = dir.path().join("validation.sh");
        {
            let mut executable_file = File::create(&executable_path).unwrap();
            executable_file.write_all(src.as_bytes()).unwrap();
            executable_file
                .set_permissions(Permissions::from_mode(0o774))
                .unwrap();
        }
        (dir, executable_path)
    }

    define_envargs_method! {envargs_correct: "ENV_IDLEMAIL_TESTARG".to_owned() => "the correct testvalue".to_owned() }
    define_envargs_method! {envargs_incorrect: "ENV_IDLEMAIL_TESTARG".to_owned() => "the incorrect testvalue".to_owned() }
    define_envargs_method! {envargs_incorrect2: "ENV_IDLEMAIL_WRONGARG".to_owned() => "the incorrect testvalue".to_owned() }

    #[test_case(vec!["ARG0", "ARG1"], envargs_correct() => true)]
    #[test_case(vec!["ARG1", "ARG0"], envargs_correct() => false)]
    #[test_case(vec!["ARG0", "ARG1"], envargs_incorrect() => false)]
    #[test_case(vec!["ARG1", "ARG0"], envargs_incorrect() => false)]
    #[test_case(vec!["ARG0", "ARG1"], envargs_incorrect2() => false)]
    #[test_case(vec!["ARG1", "ARG0"], envargs_incorrect2() => false)]
    #[test_case(vec!["ARG0"], HashMap::new() => false)]
    fn test_successfull_execution(cliargs: Vec<&str>, env: HashMap<String, String>) -> bool {
        let mail = create_testmail("unit-test source 0".to_owned());
        let mailmd5 = format!("{:x}", md5::compute(&mail.data));
        let (_dir, executable_path) = prepare_validation_script(&format!(
            r#"#!/bin/bash
            cd $(dirname "$0")
            if [ "$1" != "ARG0" ]; then echo "Argument0 incorrect"; exit 1; fi
            if [ "$2" != "ARG1" ]; then echo "Argument1 incorrect"; exit 1; fi
            if [ "$IDLEMAIL_SOURCE" != "unit-test source 0" ]; then echo "Source ENV-Param incorrect"; exit 1; fi
            if [ "$IDLEMAIL_DESTINATION" != "unit-test exec dst" ]; then echo "Destination ENV-param incorrect"; exit 1; fi
            if [ "$ENV_IDLEMAIL_TESTARG" != "the correct testvalue" ]; then echo "Wrong custom env argument"; exit 1; fi
            BODY_MD5=$(cat | md5sum | awk '{{ print $1 }}');
            if [ "$BODY_MD5" != "{}" ]; then echo "Mail body Hash-Mismatch"; exit 1; fi
        "#,
            mailmd5
        ));

        let mut execdst = ExecDestination::new(
            "unit-test exec dst".to_owned(),
            &ExecDestinationConfig {
                executable: executable_path.to_string_lossy().to_string(),
                arguments: Some(cliargs.into_iter().map(|s| s.to_owned()).collect()),
                environment: Some(env),
            },
        );
        let (ra_send, ra_recv) = mpsc::channel();
        {
            let (dst_send, dst_recv) = mpsc::channel();
            let dstchan = HubDestinationChannel {
                name: "unit-test exec dst".to_owned(),
                sender: ra_send,
                recv: dst_recv,
            };
            execdst.start(dstchan);
            dst_send.send(DestinationMessage::Mail { mail }).unwrap();
        } // drop dst_send here, this signals the destination to exit
        execdst.join();
        let res = ra_recv.try_recv();
        res.is_err()
    }
}
