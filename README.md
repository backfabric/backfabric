# backfabric

The fabric behind the interface: a network of agents.

An agent is a key, a definition and its own state. Agents share nothing and
talk only through messages. Where an agent runs is not part of its logic.

Status: early development. Stage 1 runs an echo agent over A2A on iroh.

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
address = "127.0.0.1"
port = 7400

[agent]
access = "public"   # mandatory; "public" is the only mode for now
```

```sh
backfabric serve --config node.toml
```

From another node, with its own key:

```sh
backfabric send --to <endpoint id> --addr 127.0.0.1:7400 hello
backfabric send --to <endpoint id> --addr 127.0.0.1:7400 '?What is your name'
backfabric send --to <endpoint id> --addr 127.0.0.1:7400 --task <task id> Ada
```

`send` prints the reply, the task state and the task id. Text starting with
`?` leaves the task waiting for input; the next message in that task completes
it. Each caller sees and continues only its own tasks.

A2A travels as HTTP/1.1 over iroh bidirectional streams on ALPN
`backfabric/a2a/0`. Nodes use no relays and no discovery: peers are dialled by
endpoint id and direct address.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
