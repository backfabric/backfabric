//! A node: an iroh endpoint serving the echo agent over A2A.

use std::{
    convert::Infallible,
    fmt, io,
    net::SocketAddr,
    num::NonZeroUsize,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use a2a::{A2AError, ListTasksRequest, ListTasksResponse, Task};
use a2a_grpc::GrpcHandler;
use a2a_pb::proto::a2a_service_server::A2aServiceServer;
use a2a_server::{DefaultRequestHandler, InMemoryTaskStore, TaskStore, task_store::TaskVersion};
use anyhow::{Context, Result};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey,
    endpoint::{Builder, Connection, PortmapperConfig, RecvStream, RelayMode, SendStream, presets},
    protocol::{AcceptError, IncomingFilterOutcome, ProtocolHandler, Router},
};
use iroh_tickets::endpoint::EndpointTicket;
use lru::LruCache;
use tokio::{
    io::{AsyncRead, AsyncWrite, Join, ReadBuf},
    sync::Semaphore,
};
use tonic::transport::{Server, server::Connected};

use crate::{
    A2A_ALPN, AgentId,
    echo::{self, Echo},
};

/// Callers whose tasks are kept; the least recently seen caller beyond this
/// loses its tasks.
pub const MAX_CALLERS: usize = 10_000;
/// Tasks a caller may create while its tasks are kept.
pub const MAX_TASKS_PER_CALLER: usize = 100;
/// Connections served at once.
pub const MAX_CONNECTIONS: usize = 1_024;
/// Requests served at once on one connection.
const MAX_REQUESTS_PER_CONNECTION: usize = 16;
/// Time limit for one request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

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
        let connections = a2a.connections.clone();
        let router = Router::builder(endpoint)
            .incoming_filter(Arc::new(move |_| {
                // Refuse before the handshake when every connection slot is taken.
                if connections.available_permits() == 0 {
                    IncomingFilterOutcome::Reject
                } else {
                    IncomingFilterOutcome::Accept
                }
            }))
            .accept(A2A_ALPN, a2a)
            .spawn();
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
/// Memory stays bounded: at most [`MAX_CALLERS`] callers keep their tasks,
/// least recently seen first out, each with at most [`MAX_TASKS_PER_CALLER`].
/// Entries outlive connections, so a task can be continued after reconnecting.
#[derive(Clone)]
struct A2a {
    node: EndpointId,
    callers: Arc<Mutex<LruCache<AgentId, Handler>>>,
    connections: Arc<Semaphore>,
}

// iroh requires `Debug` on protocol handlers; the SDK's handler has none.
impl fmt::Debug for A2a {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("A2a").field("node", &self.node).finish()
    }
}

impl A2a {
    fn new(node: EndpointId) -> Self {
        let max_callers = NonZeroUsize::new(MAX_CALLERS).expect("MAX_CALLERS is not zero");
        Self {
            node,
            callers: Arc::new(Mutex::new(LruCache::new(max_callers))),
            connections: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
        }
    }

    fn handler_for(&self, caller: AgentId) -> Handler {
        let mut callers = self.callers.lock().expect("caller map poisoned");
        let node = self.node;
        callers
            .get_or_insert(caller, || {
                let handler = DefaultRequestHandler::new(Echo, CappedTaskStore::default())
                    .with_extended_agent_card(echo::card(node));
                Arc::new(handler)
            })
            .clone()
    }
}

impl ProtocolHandler for A2a {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let Ok(_slot) = self.connections.clone().try_acquire_owned() else {
            conn.close(0u32.into(), b"too many connections");
            return Ok(());
        };
        // The remote id is verified by the QUIC handshake.
        let handler = self.handler_for(conn.remote_id());
        let service = A2aServiceServer::new(GrpcHandler::new(handler));
        let streams = futures::stream::unfold(conn, |conn| async move {
            let (send, recv) = conn.accept_bi().await.ok()?;
            let stream = BiStream(tokio::io::join(recv, send));
            Some((Ok::<_, Infallible>(stream), conn))
        });
        if let Err(err) = Server::builder()
            .concurrency_limit_per_connection(MAX_REQUESTS_PER_CONNECTION)
            .timeout(REQUEST_TIMEOUT)
            .add_service(service)
            .serve_with_incoming(streams)
            .await
        {
            eprintln!("grpc error: {err}");
        }
        Ok(())
    }
}

/// The SDK's in-memory task store, refusing new tasks beyond
/// [`MAX_TASKS_PER_CALLER`]; the SDK store has no limit of its own.
#[derive(Default)]
struct CappedTaskStore {
    tasks: InMemoryTaskStore,
    created: AtomicUsize,
}

#[async_trait::async_trait]
impl TaskStore for CappedTaskStore {
    async fn create(&self, task: Task) -> Result<TaskVersion, A2AError> {
        if self.created.fetch_add(1, Ordering::SeqCst) >= MAX_TASKS_PER_CALLER {
            self.created.fetch_sub(1, Ordering::SeqCst);
            return Err(A2AError::invalid_request(format!(
                "task limit of {MAX_TASKS_PER_CALLER} reached for this caller"
            )));
        }
        self.tasks.create(task).await
    }

    async fn update(&self, task: Task) -> Result<TaskVersion, A2AError> {
        self.tasks.update(task).await
    }

    async fn get(&self, task_id: &str) -> Result<Option<Task>, A2AError> {
        self.tasks.get(task_id).await
    }

    async fn list(&self, req: &ListTasksRequest) -> Result<ListTasksResponse, A2AError> {
        self.tasks.list(req).await
    }

    async fn begin_cancel(&self, task_id: &str) -> Result<Task, A2AError> {
        self.tasks.begin_cancel(task_id).await
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
