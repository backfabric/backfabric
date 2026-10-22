use std::path::PathBuf;

use anyhow::Result;
use backfabric::{
    client::{self, Client},
    config::Config,
    key::load_node_key,
    node::{Node, relay_ticket},
};
use clap::{Parser, Subcommand};
use iroh_tickets::endpoint::EndpointTicket;

/// backfabric: a network of agents.
///
/// The node key is read through secretspec (secret BACKFABRIC_NODE_KEY).
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the public key of this node, which is also its endpoint id, and
    /// its ticket when the config names a relay.
    Id {
        /// Path to the TOML configuration.
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Serve the echo agent and print its ticket once the relay is reached.
    Serve {
        /// Path to the TOML configuration.
        #[arg(long)]
        config: PathBuf,
    },
    /// Send a text message to an agent and print its reply.
    Send {
        /// Ticket of the agent, as printed by `serve`.
        #[arg(long)]
        to: EndpointTicket,
        /// Continue this task instead of starting a new one.
        #[arg(long)]
        task: Option<String>,
        /// The message text.
        text: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Id { config } => {
            let relay = match config {
                Some(path) => Config::load(&path)?.agent.relay,
                None => None,
            };
            let id = load_node_key()?.public();
            println!("public key: {id}");
            if let Some(relay) = relay {
                println!("ticket: {}", relay_ticket(id, relay));
            }
        }
        Command::Serve { config } => {
            let config = Config::load(&config)?;
            let node_key = load_node_key()?;
            let node = Node::spawn(node_key, config.bind_addr(), config.relay_mode()).await?;
            println!("endpoint id: {}", node.id());
            println!("listening on: {}", node.addr());
            println!("agent access: {}", config.agent.access);
            tokio::select! {
                ticket = node.ticket() => {
                    println!("ticket: {}", ticket?);
                    tokio::signal::ctrl_c().await?;
                }
                stop = tokio::signal::ctrl_c() => stop?,
            }
            node.shutdown().await?;
        }
        Command::Send { to, task, text } => {
            let client = Client::connect(load_node_key()?, &to).await?;
            let reply = client.send(&text, task.as_deref()).await?;
            println!("{}", client::reply(&reply));
            println!("state: {:?}", reply.status.state);
            println!("task: {}", reply.id);
            client.close().await;
        }
    }
    Ok(())
}
