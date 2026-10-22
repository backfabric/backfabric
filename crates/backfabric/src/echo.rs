//! The echo agent: replies with the text it receives.
//!
//! Text starting with `?` is a question: the task waits for input, and the
//! next message in the same task completes it with the answer.

use a2a::{
    A2AError, AgentCapabilities, AgentCard, AgentInterface, AgentSkill, Message, Part, Role,
    StreamResponse, TRANSPORT_PROTOCOL_JSONRPC, TaskState, TaskStatus, TaskStatusUpdateEvent,
};
use a2a_server::{AgentExecutor, ExecutorContext};
use futures::stream::{self, BoxStream, StreamExt};

type Events = BoxStream<'static, Result<StreamResponse, A2AError>>;

/// Base URL of the agent. The host is a placeholder: requests travel over an
/// iroh connection that is already bound to the agent's endpoint id.
pub const AGENT_URL: &str = "http://backfabric/";

/// The agent card of the echo agent.
pub fn card() -> AgentCard {
    let text = vec!["text/plain".to_string()];
    AgentCard {
        name: "echo".into(),
        description:
            "Replies with the text it receives; text starting with `?` asks for an answer first."
                .into(),
        version: env!("CARGO_PKG_VERSION").into(),
        supported_interfaces: vec![AgentInterface::new(AGENT_URL, TRANSPORT_PROTOCOL_JSONRPC)],
        capabilities: AgentCapabilities::default(),
        default_input_modes: text.clone(),
        default_output_modes: text,
        skills: vec![AgentSkill {
            id: "echo".into(),
            name: "Echo".into(),
            description: "Replies with the same text.".into(),
            tags: vec!["echo".into()],
            examples: Some(vec!["hello".into(), "?What is your name".into()]),
            input_modes: None,
            output_modes: None,
            security_requirements: None,
        }],
        provider: None,
        documentation_url: None,
        icon_url: None,
        security_schemes: None,
        security_requirements: None,
        signatures: None,
    }
}

/// Executes echo turns. It keeps no state: each turn is decided from the
/// stored task.
pub struct Echo;

impl AgentExecutor for Echo {
    fn execute(&self, ctx: ExecutorContext) -> Events {
        let text = ctx.message.as_ref().and_then(Message::text).unwrap_or("");
        let waiting = ctx
            .stored_task
            .as_ref()
            .filter(|task| task.status.state == TaskState::InputRequired);
        let (state, reply) = match waiting {
            Some(task) => {
                let question = task.status.message.as_ref().and_then(Message::text);
                let reply = format!("{} -> {text}", question.unwrap_or_default());
                (TaskState::Completed, reply)
            }
            None => match text.strip_prefix('?') {
                Some(question) => (TaskState::InputRequired, question.to_string()),
                None => (TaskState::Completed, text.to_string()),
            },
        };
        status(&ctx, state, reply)
    }

    fn cancel(&self, ctx: ExecutorContext) -> Events {
        status(&ctx, TaskState::Canceled, "canceled".into())
    }
}

/// A single status update carrying an agent message.
fn status(ctx: &ExecutorContext, state: TaskState, text: String) -> Events {
    let (task_id, context_id) = ctx.task_info();
    let mut message = Message::new(Role::Agent, vec![Part::text(text)]);
    message.task_id = Some(task_id.clone());
    message.context_id = Some(context_id.clone());
    let event = StreamResponse::StatusUpdate(TaskStatusUpdateEvent {
        task_id,
        context_id,
        status: TaskStatus {
            state,
            message: Some(message),
            timestamp: None,
        },
        metadata: None,
    });
    stream::iter([Ok(event)]).boxed()
}
