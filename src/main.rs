mod hub;
mod config;
mod destinations;
mod sources;

use log::{info, debug, error};
use signal::{trap::Trap, Signal};
use std::time::{Instant, Duration};

fn main() {
    pretty_env_logger::init();

    // dirty hack to read the config-file name from first argument for now
    let config_file = std::env::args().skip(1).take(1).next().unwrap();

    info!(target: "Idlemail", "Parsing configuration file");
    let config = match config::ConfigContainer::from_file(&config_file) {
        Ok(config) => config,
        Err(err) => {
            error!(target: "Idlemail", "Failed to parse configuration file: {}\n{}", &config_file, err);
            panic!();
        }
    };
    let mut mailhub = hub::MailHub::from_config(&config);

    #[cfg(target_os = "linux")]
    {
        debug!(target: "Idlemail", "Registering Signal traps (INT, TERM)");
        let trap = Trap::trap(&[Signal::SIGINT, Signal::SIGTERM]);
        let stop_token = mailhub.get_stop_sender();
        debug!(target: "Idlemail", "Starting signal observer thread");
        std::thread::spawn(move || {
            loop {
                match trap.wait(Instant::now() + Duration::from_millis(50)) {
                    Some(Signal::SIGINT) | Some(Signal::SIGTERM) => {
                        info!(target: "Idlemail", "Received termination signal");
                        info!(target: "Idlemail", "Initiating shutdown");
                        stop_token.stop();
                        return;
                    },
                    _ => {}
                }
            }
        });
    }

    mailhub.run();
}