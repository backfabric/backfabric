//! Two nodes in one process on loopback: a server node with the echo agent
//! and callers with their own keys. No relay is reachable offline, so the
//! tickets carry the server's loopback address instead.

use a2a::{CancelTaskRequest, GetTaskRequest, ListTasksRequest, Task, TaskState};
use backfabric::{
    client::{Client, reply},
    node::{MAX_TASKS_PER_CALLER, Node},
};
use iroh::{EndpointAddr, SecretKey, endpoint::RelayMode};
use iroh_tickets::endpoint::EndpointTicket;

async fn server() -> Node {
    let bind = "127.0.0.1:0".parse().unwrap();
    Node::spawn(SecretKey::generate(), bind, RelayMode::Disabled)
        .await
        .unwrap()
}

async fn caller(node: &Node) -> Client {
    caller_with_key(node, SecretKey::generate()).await
}

async fn caller_with_key(node: &Node, key: SecretKey) -> Client {
    let ticket = EndpointTicket::new(EndpointAddr::new(node.id()).with_ip_addr(node.addr()));
    Client::connect(key, &ticket).await.unwrap()
}

async fn get_task(client: &Client, id: &str) -> Result<Task, a2a::A2AError> {
    client
        .a2a()
        .get_task(&GetTaskRequest {
            id: id.to_string(),
            history_length: None,
            tenant: None,
        })
        .await
}

async fn list_tasks(client: &Client, status: Option<TaskState>) -> Vec<String> {
    let request = ListTasksRequest {
        context_id: None,
        status,
        page_size: None,
        page_token: None,
        history_length: None,
        status_timestamp_after: None,
        include_artifacts: None,
        tenant: None,
    };
    let response = client.a2a().list_tasks(&request).await.unwrap();
    response.tasks.into_iter().map(|task| task.id).collect()
}

#[tokio::test]
async fn echoes_text_and_completes() {
    let node = server().await;
    let client = caller(&node).await;

    let task = client.send("hello", None).await.unwrap();

    assert_eq!(task.status.state, TaskState::Completed);
    assert_eq!(reply(&task), "hello");
    let stored = get_task(&client, &task.id).await.unwrap();
    assert_eq!(stored.status.state, TaskState::Completed);

    client.close().await;
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn card_names_the_agent_by_its_iroh_endpoint() {
    let node = server().await;
    let client = caller(&node).await;

    let interfaces = &client.card().supported_interfaces;

    assert_eq!(interfaces.len(), 1);
    assert_eq!(interfaces[0].url, format!("iroh://{}", node.id()));
    assert_eq!(interfaces[0].protocol_binding, "GRPC");

    client.close().await;
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn question_waits_for_input_then_completes_with_answer() {
    let node = server().await;
    let client = caller(&node).await;

    let asked = client.send("?What is your name", None).await.unwrap();
    assert_eq!(asked.status.state, TaskState::InputRequired);
    assert_eq!(reply(&asked), "What is your name");
    assert_eq!(
        list_tasks(&client, Some(TaskState::InputRequired)).await,
        [asked.id.as_str()]
    );

    let answered = client.send("Ada", Some(&asked.id)).await.unwrap();
    assert_eq!(answered.id, asked.id);
    assert_eq!(answered.status.state, TaskState::Completed);
    assert!(reply(&answered).contains("Ada"), "{}", reply(&answered));
    assert!(
        list_tasks(&client, Some(TaskState::InputRequired))
            .await
            .is_empty()
    );

    client.close().await;
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn task_continues_after_the_caller_reconnects() {
    let node = server().await;
    let key = SecretKey::generate();
    let first = caller_with_key(&node, key.clone()).await;
    let asked = first.send("?Your name", None).await.unwrap();
    first.close().await;

    let second = caller_with_key(&node, key).await;
    let answered = second.send("Ada", Some(&asked.id)).await.unwrap();

    assert_eq!(answered.status.state, TaskState::Completed);
    assert!(reply(&answered).contains("Ada"), "{}", reply(&answered));

    second.close().await;
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn caller_cannot_create_tasks_beyond_its_limit() {
    let node = server().await;
    let greedy = caller(&node).await;
    let other = caller(&node).await;

    let mut waiting = None;
    for n in 0..MAX_TASKS_PER_CALLER {
        let task = greedy.send(&format!("?{n}"), None).await.unwrap();
        waiting = Some(task.id);
    }
    let refused = greedy.send("one too many", None).await;

    assert!(refused.is_err());
    // Existing tasks still continue, and other callers are not affected.
    let answered = greedy.send("answer", waiting.as_deref()).await.unwrap();
    assert_eq!(answered.status.state, TaskState::Completed);
    let task = other.send("hello", None).await.unwrap();
    assert_eq!(task.status.state, TaskState::Completed);

    greedy.close().await;
    other.close().await;
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn callers_see_and_continue_only_their_own_tasks() {
    let node = server().await;
    let alice = caller(&node).await;
    let bob = caller(&node).await;

    let task = alice.send("?Secret question", None).await.unwrap();
    assert_eq!(task.status.state, TaskState::InputRequired);

    assert!(get_task(&bob, &task.id).await.is_err());
    assert!(list_tasks(&bob, None).await.is_empty());
    assert!(bob.send("Mallory's answer", Some(&task.id)).await.is_err());

    // Bob's attempts left Alice's task untouched.
    let stored = get_task(&alice, &task.id).await.unwrap();
    assert_eq!(stored.status.state, TaskState::InputRequired);
    let answered = alice.send("Alice's answer", Some(&task.id)).await.unwrap();
    assert_eq!(answered.status.state, TaskState::Completed);
    assert!(reply(&answered).contains("Alice's answer"));

    alice.close().await;
    bob.close().await;
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn caller_cannot_cancel_another_callers_task() {
    let node = server().await;
    let alice = caller(&node).await;
    let bob = caller(&node).await;
    let task = alice.send("?Secret question", None).await.unwrap();

    let canceled = bob
        .a2a()
        .cancel_task(&CancelTaskRequest {
            id: task.id.clone(),
            metadata: None,
            tenant: None,
        })
        .await;

    assert!(canceled.is_err());
    let stored = get_task(&alice, &task.id).await.unwrap();
    assert_eq!(stored.status.state, TaskState::InputRequired);

    alice.close().await;
    bob.close().await;
    node.shutdown().await.unwrap();
}
