//! The `backfabric` binary: startup errors and what `id` prints.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

use iroh::{RelayUrl, SecretKey};
use iroh_tickets::endpoint::EndpointTicket;
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_backfabric");

fn write_config(dir: &TempDir, text: &str) -> PathBuf {
    let path = dir.path().join("node.toml");
    std::fs::write(&path, text).unwrap();
    path
}

fn serve(config: &str) -> Output {
    let dir = tempfile::tempdir().unwrap();
    Command::new(BIN)
        .arg("serve")
        .arg("--config")
        .arg(write_config(&dir, config))
        .output()
        .unwrap()
}

fn id(key: &str, config: Option<&Path>) -> Output {
    let mut command = Command::new(BIN);
    command.arg("id");
    if let Some(config) = config {
        command.arg("--config").arg(config);
    }
    command
        .env("SECRETSPEC_PROVIDER", "env")
        .env("BACKFABRIC_NODE_KEY", key)
        .output()
        .unwrap()
}

fn stdout_field<'a>(stdout: &'a str, label: &str) -> &'a str {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(label))
        .unwrap_or_else(|| panic!("no {label:?} in {stdout:?}"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn serve_refuses_config_without_access() {
    let output = serve("[node]\naddress = \"127.0.0.1\"\nport = 0\n\n[agent]\n");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("missing field `access`"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn serve_refuses_access_other_than_public() {
    let output = serve("[node]\naddress = \"127.0.0.1\"\nport = 0\n\n[agent]\naccess = \"pass\"\n");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("unknown variant `pass`"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn id_prints_the_public_key_of_the_node_key() {
    let key = "11".repeat(32);
    let expected: SecretKey = key.parse().unwrap();

    let output = id(&key, None);

    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout_field(&stdout, "public key: "),
        expected.public().to_string()
    );
    assert!(!stdout.contains("ticket"), "{stdout}");
}

#[test]
fn id_accepts_uppercase_hex_as_the_same_key() {
    let lower = id(&"ab".repeat(32), None);
    let upper = id(&"AB".repeat(32), None);

    assert!(upper.status.success(), "{}", stderr(&upper));
    assert_eq!(upper.stdout, lower.stdout);
}

#[test]
fn id_prints_a_ticket_with_the_relay_and_no_ip_addresses() {
    let key = "22".repeat(32);
    let expected: SecretKey = key.parse().unwrap();
    let relay: RelayUrl = "https://relay.example.org".parse().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config = write_config(
        &dir,
        &format!(
            "[node]\naddress = \"127.0.0.1\"\nport = 7400\n\n\
             [agent]\naccess = \"public\"\nrelay = \"{relay}\"\n"
        ),
    );

    let output = id(&key, Some(&config));

    assert!(output.status.success(), "{}", stderr(&output));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let ticket: EndpointTicket = stdout_field(&stdout, "ticket: ").parse().unwrap();
    let addr = ticket.endpoint_addr();
    assert_eq!(addr.id, expected.public());
    assert_eq!(addr.relay_urls().collect::<Vec<_>>(), [&relay]);
    assert_eq!(addr.ip_addrs().count(), 0);
}

#[test]
fn id_refuses_key_that_is_not_64_hex_characters() {
    for key in ["abc", &"zz".repeat(32), &"11".repeat(33)] {
        let output = id(key, None);

        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("must be 64 hex characters"),
            "{}",
            stderr(&output)
        );
    }
}
