//! An A2A client that reaches agents over iroh with the caller's node key.

use std::sync::Arc;

use a2a::{Message, Part, Role, SendMessageRequest, SendMessageResponse, Task};
use a2a_client::{
    A2AClient, A2AClientFactory, Transport, agent_card::AgentCardResolver,
    jsonrpc::JsonRpcTransportFactory,
};
use anyhow::{Context, Result, bail};
use iroh::{Endpoint, SecretKey, endpoint::Connection, endpoint::RelayMode};
use iroh_tickets::endpoint::EndpointTicket;
use tempfile::TempDir;
use tokio::net::UnixListener;
use tokio_util::task::AbortOnDropHandle;

use crate::{A2A_ALPN, echo::AGENT_URL, node::endpoint_builder};

/// A connection to one remote agent.
pub struct Client {
    a2a: A2AClient<Box<dyn Transport>>,
    endpoint: Endpoint,
    _bridge: AbortOnDropHandle<()>,
    _socket_dir: TempDir,
}

impl Client {
    /// Connects with `key` to the agent addressed by `ticket` and fetches its
    /// agent card. The only relay used is the one in the ticket.
    pub async fn connect(key: SecretKey, ticket: &EndpointTicket) -> Result<Self> {
        let to = ticket.endpoint_addr().clone();
        let relays: Vec<_> = to.relay_urls().cloned().collect();
        let relay = if relays.is_empty() {
            RelayMode::Disabled
        } else {
            RelayMode::custom(relays)
        };
        let endpoint = endpoint_builder(key, relay).bind().await?;
        let id = to.id;
        let conn = endpoint
            .connect(to, A2A_ALPN)
            .await
            .with_context(|| format!("cannot connect to {id}"))?;

        // reqwest cannot dial an iroh stream (its connection type is sealed), so
        // a private Unix socket bridges each HTTP connection to a new stream.
        let socket_dir = tempfile::tempdir()?;
        let socket = socket_dir.path().join("a2a.sock");
        let listener = UnixListener::bind(&socket)?;
        let bridge = AbortOnDropHandle::new(tokio::spawn(bridge(listener, conn)));
        let http = reqwest::Client::builder()
            .unix_socket(socket)
            .http1_only()
            .build()?;

        let card = AgentCardResolver::new(Some(http.clone()))
            .resolve(AGENT_URL)
            .await?;
        let a2a = A2AClientFactory::builder()
            .no_defaults()
            .register(Arc::new(JsonRpcTransportFactory::new(Some(http))))
            .build()
            .create_from_card(&card)
            .await?;
        Ok(Self {
            a2a,
            endpoint,
            _bridge: bridge,
            _socket_dir: socket_dir,
        })
    }

    /// The official A2A client, for the methods this type does not wrap.
    pub fn a2a(&self) -> &A2AClient<Box<dyn Transport>> {
        &self.a2a
    }

    /// Sends `text`, as a new task or into the existing task `task`, and
    /// returns the task as the agent left it.
    pub async fn send(&self, text: &str, task: Option<&str>) -> Result<Task> {
        let mut message = Message::new(Role::User, vec![Part::text(text)]);
        message.task_id = task.map(str::to_string);
        let request = SendMessageRequest {
            message,
            configuration: None,
            metadata: None,
            tenant: None,
        };
        match self.a2a.send_message(&request).await? {
            SendMessageResponse::Task(task) => Ok(task),
            SendMessageResponse::Message(_) => bail!("the agent replied without a task"),
        }
    }

    /// Closes the connection gracefully.
    pub async fn close(self) {
        self.endpoint.close().await;
    }
}

/// The text of the agent's latest status message in `task`.
pub fn reply(task: &Task) -> &str {
    task.status
        .message
        .as_ref()
        .and_then(Message::text)
        .unwrap_or_default()
}

/// Forwards every connection accepted on `listener` to a new bidirectional
/// stream of `conn`.
async fn bridge(listener: UnixListener, conn: Connection) {
    while let Ok((mut local, _)) = listener.accept().await {
        let conn = conn.clone();
        tokio::spawn(async move {
            // A failed stream surfaces to the HTTP client as a closed connection.
            if let Ok((send, recv)) = conn.open_bi().await {
                let mut remote = tokio::io::join(recv, send);
                let _ = tokio::io::copy_bidirectional(&mut local, &mut remote).await;
            }
        });
    }
}
