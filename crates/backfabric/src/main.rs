use std::{net::SocketAddr, path::PathBuf};

use anyhow::Result;
use backfabric::{
    client::{self, Client},
    config::Config,
    key::load_node_key,
    node::Node,
};
use clap::{Parser, Subcommand};
use iroh::EndpointId;

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
    /// Print the public key of this node, which is also its endpoint id.
    Id,
    /// Serve the echo agent.
    Serve {
        /// Path to the TOML configuration.
        #[arg(long)]
        config: PathBuf,
    },
    /// Send a text message to an agent and print its reply.
    Send {
        /// Endpoint id of the agent's node.
        #[arg(long)]
        to: EndpointId,
        /// Direct address of the agent's node.
        #[arg(long)]
        addr: SocketAddr,
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
        Command::Id => println!("{}", load_node_key()?.public()),
        Command::Serve { config } => {
            let config = Config::load(&config)?;
            let node = Node::spawn(load_node_key()?, &config).await?;
            println!("endpoint id: {}", node.id());
            println!("listening on: {}", node.addr());
            println!("agent access: {}", config.agent.access);
            tokio::signal::ctrl_c().await?;
            node.shutdown().await?;
        }
        Command::Send {
            to,
            addr,
            task,
            text,
        } => {
            let client = Client::connect(load_node_key()?, to, addr).await?;
            let reply = client.send(&text, task.as_deref()).await?;
            println!("{}", client::reply(&reply));
            println!("state: {:?}", reply.status.state);
            println!("task: {}", reply.id);
            client.close().await;
        }
    }
    Ok(())
}
