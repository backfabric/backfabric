# backfabric

The fabric behind the interface: a network of agents.

An agent is a key, a definition and its own state. Agents share nothing and
talk only through messages. Where an agent runs is not part of its logic.

Status: early development. A node runs an echo agent over A2A on iroh.

## Usage

A node is identified by an Ed25519 key. backfabric reads it through
[secretspec](https://secretspec.dev) as `BACKFABRIC_NODE_KEY` (see
`secretspec.toml`): 32 bytes as 64 hex characters. backfabric never generates
keys; create one yourself, for example with `openssl rand -hex 32`, and give
it to the secretspec provider of your choice. With the environment provider:

```sh
export SECRETSPEC_PROVIDER=env
export BACKFABRIC_NODE_KEY=$(openssl rand -hex 32)
backfabric id    # prints the public key, which is also the endpoint id
```

Serve the echo agent with a config file:

```toml
[node]
address = "0.0.0.0"
port = 7400

[agent]
access = "public"   # mandatory; "public" is the only mode for now
# relay = "https://relay.example.org"   # optional; iroh's n0 relays otherwise
```

```sh
backfabric serve --config node.toml
```

Once the node reaches its home relay, `serve` prints the agent's ticket: an
iroh endpoint ticket with the endpoint id and the relay URL, and no IP
addresses. With a relay in the config, `backfabric id --config node.toml`
prints the same ticket without starting the node.

From another node, with its own key:

```sh
backfabric send --to <ticket> hello
backfabric send --to <ticket> '?What is your name'
backfabric send --to <ticket> --task <task id> Ada
```

`send` prints the reply, the task state and the task id. Text starting with
`?` leaves the task waiting for input; the next message in that task completes
it. Each caller sees and continues only its own tasks.

A2A uses its gRPC binding over an iroh bidirectional stream on ALPN
`backfabric/a2a-grpc/0`. The agent card lists one interface,
`iroh://<endpoint id>` with the GRPC binding, and is served through
GetExtendedAgentCard. Nodes are reached through a relay and publish nothing:
no pkarr or DNS discovery and no address lookup, so agents learn about each
other only from tickets.

A public agent keeps its memory bounded: the tasks of at most 10,000 callers
are kept (least recently seen dropped first), each caller may create at most
100 tasks, at most 1,024 connections are served at once, and each request has
a 30-second time limit.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
