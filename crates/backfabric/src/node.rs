//! A node: an iroh endpoint serving the echo agent over A2A.

use std::{
    collections::HashMap,
    convert::Infallible,
    fmt, io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context as TaskContext, Poll},
};

use a2a_grpc::GrpcHandler;
use a2a_pb::proto::a2a_service_server::A2aServiceServer;
use a2a_server::{DefaultRequestHandler, InMemoryTaskStore};
use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey,
    endpoint::{Builder, Connection, PortmapperConfig, RecvStream, RelayMode, SendStream, presets},
    protocol::{AcceptError, ProtocolHandler, Router},
};
use iroh_tickets::endpoint::EndpointTicket;
use tokio::io::{AsyncRead, AsyncWrite, Join, ReadBuf};
use tonic::transport::{Server, server::Connected};

use crate::{
    A2A_ALPN, AgentId,
    echo::{self, Echo},
};

/// An endpoint builder that publishes nothing: no address lookup, no pkarr or
/// DNS discovery, no port mapping. Peers are reached through `relay` or by
/// direct address.
pub(crate) fn endpoint_builder(node_key: SecretKey, relay: RelayMode) -> Builder {
    Endpoint::builder(presets::Minimal)
        .secret_key(node_key)
        .relay_mode(relay)
        .portmapper_config(PortmapperConfig::Disabled)
}

/// The address of an agent: its endpoint id and its relay, no IP addresses.
pub fn relay_ticket(id: EndpointId, relay: RelayUrl) -> EndpointTicket {
    EndpointTicket::new(EndpointAddr::new(id).with_relay_url(relay))
}

/// A running node.
pub struct Node {
    router: Router,
}

impl Node {
    /// Binds the endpoint with `node_key` at `bind`, using `relay`, and starts
    /// serving the agent.
    pub async fn spawn(node_key: SecretKey, bind: SocketAddr, relay: RelayMode) -> Result<Self> {
        let endpoint = endpoint_builder(node_key, relay)
            .clear_ip_transports()
            .bind_addr(bind)?
            .bind()
            .await
            .with_context(|| format!("cannot bind {bind}"))?;
        let a2a = A2a::new(endpoint.id());
        let router = Router::builder(endpoint).accept(A2A_ALPN, a2a).spawn();
        Ok(Self { router })
    }

    /// The endpoint id, which is also the public node key.
    pub fn id(&self) -> EndpointId {
        self.router.endpoint().id()
    }

    /// Waits until the node is connected to its home relay and returns its
    /// ticket.
    pub async fn ticket(&self) -> Result<EndpointTicket> {
        let endpoint = self.router.endpoint();
        endpoint.online().await;
        let relay = endpoint
            .addr()
            .relay_urls()
            .next()
            .cloned()
            .context("the node has no home relay")?;
        Ok(relay_ticket(self.id(), relay))
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

type Handler = Arc<DefaultRequestHandler>;

/// A2A over iroh: a gRPC server on the bidirectional streams of a connection.
///
/// Every caller gets its own request handler and task store, chosen by the
/// caller's verified key, so a caller sees and continues only its own tasks.
#[derive(Clone)]
struct A2a {
    node: EndpointId,
    callers: Arc<Mutex<HashMap<AgentId, Handler>>>,
}

// iroh requires `Debug` on protocol handlers; the SDK's handler has none.
impl fmt::Debug for A2a {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("A2a").field("node", &self.node).finish()
    }
}

impl A2a {
    fn new(node: EndpointId) -> Self {
        Self {
            node,
            callers: Arc::default(),
        }
    }

    fn handler_for(&self, caller: AgentId) -> Handler {
        let mut callers = self.callers.lock().expect("caller map poisoned");
        let node = self.node;
        callers
            .entry(caller)
            .or_insert_with(|| {
                let handler = DefaultRequestHandler::new(Echo, InMemoryTaskStore::new())
                    .with_extended_agent_card(echo::card(node));
                Arc::new(handler)
            })
            .clone()
    }
}

impl ProtocolHandler for A2a {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        // The remote id is verified by the QUIC handshake.
        let handler = self.handler_for(conn.remote_id());
        let service = A2aServiceServer::new(GrpcHandler::new(handler));
        let streams = futures::stream::unfold(conn, |conn| async move {
            let (send, recv) = conn.accept_bi().await.ok()?;
            let stream = BiStream(tokio::io::join(recv, send));
            Some((Ok::<_, Infallible>(stream), conn))
        });
        if let Err(err) = Server::builder()
            .add_service(service)
            .serve_with_incoming(streams)
            .await
        {
            eprintln!("grpc error: {err}");
        }
        Ok(())
    }
}

/// One iroh bidirectional stream as a single byte stream.
///
/// tonic accepts only streams that implement its `Connected` trait, and
/// neither tonic nor iroh implements it for iroh streams.
struct BiStream(Join<RecvStream, SendStream>);

impl Connected for BiStream {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl AsyncRead for BiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for BiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
