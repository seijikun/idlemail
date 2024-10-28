use serde_derive::{Deserialize, Serialize};
use std::{collections::HashMap, fs::File, io::BufReader, path::Path};

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ConfigContainer {
    pub destinations: HashMap<String, DestinationConfig>,
    pub sources: HashMap<String, SourceConfig>,
    pub retryagent: Option<RetryAgentConfig>,
    pub mappings: HashMap<String, Vec<String>>,
}
impl ConfigContainer {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<ConfigContainer, String> {
        let config_file = File::open(path).map_err(|_| "Failed to open config file".to_owned())?;
        let reader = BufReader::new(config_file);
        let config: ConfigContainer = serde_json::from_reader(reader)
            .map_err(|e| format!("Failed to parse config file: {}", e))?;
        config.validate()?;
        Ok(config)
    }
    fn validate(&self) -> Result<(), String> {
        for (srcname, dsts) in &self.mappings {
            if !self.sources.contains_key(srcname) {
                return Err(format!("Unknown source: {} specified in mappings", srcname));
            }
            for dstname in dsts {
                if !self.destinations.contains_key(dstname) {
                    return Err(format!(
                        "Unknown destination: {} specified in mappings",
                        dstname
                    ));
                }
            }
        }
        for srcname in self.sources.keys() {
            if !self.mappings.contains_key(srcname) {
                return Err(format!("Source: {} has no mapping", srcname));
            }
        }
        if let Some(RetryAgentConfig::Filesystem(config)) = &self.retryagent {
            if !Path::new(&config.path).exists() {
                return Err("FilesystemRetryAgent: Path does not exist".to_string());
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum AuthMethod {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "plain")]
    Plain { user: String, password: String },
    #[serde(rename = "login")]
    Login { user: String, password: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum Encryption {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "ssl")]
    Ssl,
    #[serde(rename = "starttls")]
    Starttls,
}

// #############
// # Sources
// #############

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ImapPollSourceConfig {
    pub server: String,
    pub port: u16,
    pub interval: u64,
    pub keep: bool,
    pub auth: AuthMethod,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ImapIdleSourceConfig {
    pub server: String,
    pub port: u16,
    pub path: String,
    pub renewinterval: u64,
    pub keep: bool,
    pub auth: AuthMethod,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct TestSourceConfig {
    pub delay: u64,
    pub interval: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum SourceConfig {
    #[serde(rename = "test")]
    Test(TestSourceConfig),
    #[serde(rename = "imap_poll")]
    ImapPoll(ImapPollSourceConfig),
    #[serde(rename = "imap_idle")]
    ImapIdle(ImapIdleSourceConfig),
}

// #############
// # Destinations
// #############

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct SmtpDestinationConfig {
    pub server: String,
    pub port: u16,
    pub encryption: Encryption,
    pub auth: Option<AuthMethod>,
    pub recipient: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct TestDestinationConfig {
    pub fail_n_first: u16,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct ExecDestinationConfig {
    pub executable: String,
    pub arguments: Option<Vec<String>>,
    pub environment: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum DestinationConfig {
    #[serde(rename = "test")]
    Test(TestDestinationConfig),
    #[serde(rename = "smtp")]
    Smtp(SmtpDestinationConfig),
    #[serde(rename = "exec")]
    Exec(ExecDestinationConfig),
}

// #############
// # RetryAgent
// #############

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct MemoryRetryAgentConfig {
    pub delay: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct FilesystemRetryAgentConfig {
    pub delay: u64,
    pub path: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
pub enum RetryAgentConfig {
    #[serde(rename = "memory")]
    Memory(MemoryRetryAgentConfig),
    #[serde(rename = "filesystem")]
    Filesystem(FilesystemRetryAgentConfig),
}
