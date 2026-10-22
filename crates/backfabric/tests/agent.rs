//! Two nodes in one process on loopback: a server node with the echo agent
//! and callers with their own keys. No relay is reachable offline, so the
//! tickets carry the server's loopback address instead.

use a2a::{GetTaskRequest, ListTasksRequest, Task, TaskState};
use backfabric::{
    client::{Client, reply},
    node::Node,
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
    let ticket = EndpointTicket::new(EndpointAddr::new(node.id()).with_ip_addr(node.addr()));
    Client::connect(SecretKey::generate(), &ticket)
        .await
        .unwrap()
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
