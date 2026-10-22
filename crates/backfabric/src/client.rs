//! An A2A client that reaches agents over iroh with the caller's node key.

use a2a::{
    AgentCard, GetExtendedAgentCardRequest, Message, Part, Role, SendMessageRequest,
    SendMessageResponse, Task,
};
use a2a_client::A2AClient;
use a2a_grpc::GrpcTransport;
use anyhow::{Context, Result, bail};
use hyper_util::rt::TokioIo;
use iroh::{Endpoint, SecretKey, endpoint::RelayMode};
use iroh_tickets::endpoint::EndpointTicket;
use tonic::transport::{Endpoint as GrpcEndpoint, Uri};

use crate::{A2A_ALPN, a2a_url, node::endpoint_builder};

/// A connection to one remote agent.
pub struct Client {
    a2a: A2AClient<GrpcTransport>,
    card: AgentCard,
    endpoint: Endpoint,
}

impl Client {
    /// Connects with `node_key` to the agent addressed by `ticket` and fetches
    /// its agent card. The only relay used is the one in the ticket.
    pub async fn connect(node_key: SecretKey, ticket: &EndpointTicket) -> Result<Self> {
        let to = ticket.endpoint_addr().clone();
        let relays: Vec<_> = to.relay_urls().cloned().collect();
        let relay = if relays.is_empty() {
            RelayMode::Disabled
        } else {
            RelayMode::custom(relays)
        };
        let endpoint = endpoint_builder(node_key, relay).bind().await?;
        let id = to.id;
        let conn = endpoint
            .connect(to, A2A_ALPN)
            .await
            .with_context(|| format!("cannot connect to {id}"))?;

        // gRPC runs over one bidirectional stream of the iroh connection,
        // opened again whenever tonic reconnects.
        let connector = tower::service_fn(move |_: Uri| {
            let conn = conn.clone();
            async move {
                let (send, recv) = conn.open_bi().await?;
                Ok::<_, iroh::endpoint::ConnectionError>(TokioIo::new(tokio::io::join(recv, send)))
            }
        });
        let channel = GrpcEndpoint::from_shared(a2a_url(id))?
            .connect_with_connector(connector)
            .await
            .with_context(|| format!("cannot open A2A to {id}"))?;
        let a2a = A2AClient::new(GrpcTransport::from_channel(channel));
        let card = a2a
            .get_extended_agent_card(&GetExtendedAgentCardRequest { tenant: None })
            .await
            .context("cannot fetch the agent card")?;
        Ok(Self {
            a2a,
            card,
            endpoint,
        })
    }

    /// The agent card of the remote agent.
    pub fn card(&self) -> &AgentCard {
        &self.card
    }

    /// The official A2A client, for the methods this type does not wrap.
    pub fn a2a(&self) -> &A2AClient<GrpcTransport> {
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
