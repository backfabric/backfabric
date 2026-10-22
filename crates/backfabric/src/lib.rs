//! backfabric: the fabric behind the interface, a network of agents.
//!
//! A node holds an Ed25519 node key and serves an agent over A2A. A2A travels
//! as gRPC over iroh streams, so every caller is identified by its verified
//! key.

use iroh::EndpointId;

pub mod client;
pub mod config;
pub mod echo;
pub mod key;
pub mod node;

/// ALPN of A2A over iroh: gRPC (HTTP/2) on a bidirectional stream.
pub const A2A_ALPN: &[u8] = b"backfabric/a2a-grpc/0";

/// The key that identifies an agent, as opposed to the node key that secures
/// the iroh transport. For now an agent's key is the key of the node it runs
/// on, so a caller's agent id is its verified endpoint id.
pub type AgentId = iroh::PublicKey;

/// The A2A interface URL of the agent served by `node`.
pub fn a2a_url(node: EndpointId) -> String {
    format!("iroh://{node}")
}
