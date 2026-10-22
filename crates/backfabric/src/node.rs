//! A node: an iroh endpoint serving the echo agent over A2A.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use a2a_server::{
    DefaultRequestHandler, InMemoryTaskStore, StaticAgentCard, agent_card::agent_card_router,
    jsonrpc::jsonrpc_router,
};
use anyhow::{Context, Result};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use iroh::{
    Endpoint, EndpointId, SecretKey,
    endpoint::{Builder, Connection, PortmapperConfig, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};

use crate::{
    A2A_ALPN,
    config::Config,
    echo::{self, Echo},
};

/// An endpoint builder that never touches the internet: no relays, no
/// address lookup, no port mapping. Peers are dialled by direct address.
pub(crate) fn endpoint_builder(key: SecretKey) -> Builder {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .portmapper_config(PortmapperConfig::Disabled)
}

/// A running node.
pub struct Node {
    router: Router,
}

impl Node {
    /// Binds the endpoint with `key` at the configured address and starts
    /// serving the agent.
    pub async fn spawn(key: SecretKey, config: &Config) -> Result<Self> {
        let endpoint = endpoint_builder(key)
            .clear_ip_transports()
            .bind_addr(config.bind_addr())?
            .bind()
            .await
            .with_context(|| format!("cannot bind {}", config.bind_addr()))?;
        let router = Router::builder(endpoint)
            .accept(A2A_ALPN, A2a::default())
            .spawn();
        Ok(Self { router })
    }

    /// The endpoint id, which is also the node's public key.
    pub fn id(&self) -> EndpointId {
        self.router.endpoint().id()
    }

    /// The address the endpoint is bound to.
    pub fn addr(&self) -> SocketAddr {
        self.router.endpoint().bound_sockets()[0]
    }

    /// Stops accepting connections and closes the endpoint.
    pub async fn shutdown(self) -> Result<()> {
        Ok(self.router.shutdown().await?)
    }
}

/// A2A over iroh: HTTP/1.1 on each bidirectional stream of a connection.
///
/// Every caller key gets its own request handler and task store, so a caller
/// sees and continues only its own tasks.
#[derive(Debug, Clone, Default)]
struct A2a {
    callers: Arc<Mutex<HashMap<EndpointId, axum::Router>>>,
}

impl A2a {
    fn app_for(&self, caller: EndpointId) -> axum::Router {
        let mut callers = self.callers.lock().expect("caller map poisoned");
        callers
            .entry(caller)
            .or_insert_with(|| {
                let handler = DefaultRequestHandler::new(Echo, InMemoryTaskStore::new());
                agent_card_router(Arc::new(StaticAgentCard::new(echo::card())))
                    .merge(jsonrpc_router(Arc::new(handler)))
            })
            .clone()
    }
}

impl ProtocolHandler for A2a {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        // The remote id is verified by the QUIC handshake.
        let app = self.app_for(conn.remote_id());
        while let Ok((send, recv)) = conn.accept_bi().await {
            let io = TokioIo::new(tokio::io::join(recv, send));
            let service = TowerToHyperService::new(app.clone());
            tokio::spawn(async move {
                if let Err(err) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    eprintln!("http error: {err}");
                }
            });
        }
        Ok(())
    }
}
