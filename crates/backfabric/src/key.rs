//! The node key: an Ed25519 secret key provided by ops through secretspec.

use anyhow::{Context, Result, bail};
use data_encoding::HEXLOWER_PERMISSIVE;
use iroh::SecretKey;
use secretspec::{NamedResolution, Secrets, Spec};
use zeroize::Zeroizing;

/// Name of the secret holding the node key.
pub const NODE_KEY: &str = "BACKFABRIC_NODE_KEY";

/// The secrets this software declares; ops chooses the provider.
const SECRETSPEC: &str = include_str!("../../../secretspec.toml");

/// Loads the node key through secretspec, using the provider chosen by ops.
pub fn load_node_key() -> Result<SecretKey> {
    let spec = Spec::from_toml(SECRETSPEC).context("invalid secretspec.toml")?;
    let secrets = Secrets::from_spec(spec).context("cannot set up secretspec")?;
    let resolution = secrets
        .resolve_named_bytes(NODE_KEY)
        .with_context(|| format!("cannot load {NODE_KEY} through secretspec"))?;
    match resolution {
        NamedResolution::Resolved(secret) => {
            let value = secret
                .value
                .with_context(|| format!("{NODE_KEY} must be an inline value, not a path"))?;
            parse_node_key(value.expose_secret())
        }
        NamedResolution::Missing { .. } => bail!(
            "{NODE_KEY} is not set; create one with `openssl rand -hex 32` \
             and provide it through secretspec"
        ),
        NamedResolution::Undeclared => bail!("{NODE_KEY} is not declared in secretspec.toml"),
    }
}

/// Parses a node key given as 64 hex characters (32 bytes), in either case.
/// The decoded bytes are wiped from memory once the key is built.
pub fn parse_node_key(hex: &[u8]) -> Result<SecretKey> {
    let hex = hex.trim_ascii();
    let mut bytes = Zeroizing::new([0u8; 32]);
    if HEXLOWER_PERMISSIVE.decode_len(hex.len()) != Ok(bytes.len())
        || HEXLOWER_PERMISSIVE.decode_mut(hex, bytes.as_mut()).is_err()
    {
        bail!(
            "{NODE_KEY} must be 64 hex characters (32 bytes), got {} characters",
            hex.len()
        );
    }
    Ok(SecretKey::from_bytes(&bytes))
}
