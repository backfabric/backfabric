//! backfabric: the fabric behind the interface, a network of agents.
//!
//! A node holds an Ed25519 key and serves an agent over A2A. A2A travels as
//! HTTP/1.1 over iroh streams, so every caller is identified by its verified
//! key.

pub mod client;
pub mod config;
pub mod echo;
pub mod key;
pub mod node;

/// ALPN of A2A over iroh: HTTP/1.1 on each bidirectional stream.
pub const A2A_ALPN: &[u8] = b"backfabric/a2a/0";
