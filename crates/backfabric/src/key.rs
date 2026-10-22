//! The node key: an Ed25519 secret key provided by ops through secretspec.

use anyhow::{Context, Result, bail};
use iroh::SecretKey;
use secretspec::{NamedResolution, Secrets, Spec};

/// Name of the secret holding the node key.
pub const NODE_KEY: &str = "BACKFABRIC_NODE_KEY";

/// The secrets this software declares; ops chooses the provider.
const SECRETSPEC: &str = include_str!("../../../secretspec.toml");

/// Loads the node key through secretspec, using the provider chosen by ops.
pub fn load_node_key() -> Result<SecretKey> {
    let spec = Spec::from_toml(SECRETSPEC).context("invalid secretspec.toml")?;
    let secrets = Secrets::from_spec(spec).context("cannot set up secretspec")?;
    let resolution = secrets
        .resolve_named(NODE_KEY)
        .with_context(|| format!("cannot load {NODE_KEY} through secretspec"))?;
    match resolution {
        NamedResolution::Resolved(secret) => {
            let value = secret
                .value
                .with_context(|| format!("{NODE_KEY} must be an inline value, not a path"))?;
            parse_node_key(&value)
        }
        NamedResolution::Missing { .. } => bail!(
            "{NODE_KEY} is not set; create one with `openssl rand -hex 32` \
             and provide it through secretspec"
        ),
        NamedResolution::Undeclared => bail!("{NODE_KEY} is not declared in secretspec.toml"),
    }
}

/// Parses a node key given as 64 hex characters (32 bytes).
pub fn parse_node_key(text: &str) -> Result<SecretKey> {
    let text = text.trim();
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!(
            "{NODE_KEY} must be 64 hex characters (32 bytes), got {} characters",
            text.chars().count()
        );
    }
    // iroh parses lowercase hex only; uppercase hex is the same key.
    Ok(text.to_ascii_lowercase().parse()?)
}
