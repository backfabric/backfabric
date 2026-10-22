//! Node configuration, read from a TOML file.

use std::{
    fmt,
    net::{IpAddr, SocketAddr},
    path::Path,
};

use anyhow::{Context, Result};
use serde::Deserialize;

/// The whole configuration of a node.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub node: NodeConfig,
    pub agent: AgentConfig,
}

/// Where the node listens.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub address: IpAddr,
    pub port: u16,
}

/// The definition of the agent served by the node.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    /// Who may call the agent. Mandatory: access is never implied.
    pub access: Access,
}

/// Access mode of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Anyone may call the agent.
    Public,
}

impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Access::Public => f.write_str("public"),
        }
    }
}

impl Config {
    /// Reads and validates the configuration file at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read config {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    /// Parses and validates a configuration from TOML text.
    pub fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }

    /// The socket address the node binds to.
    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(self.node.address, self.node.port)
    }
}
