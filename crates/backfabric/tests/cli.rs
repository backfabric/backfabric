//! The `backfabric` binary refuses bad startup input with a clear error.

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_backfabric");

fn serve(config: &str) -> Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("node.toml");
    std::fs::write(&path, config).unwrap();
    Command::new(BIN)
        .arg("serve")
        .arg("--config")
        .arg(&path)
        .output()
        .unwrap()
}

fn id(key: &str) -> Output {
    Command::new(BIN)
        .arg("id")
        .env("SECRETSPEC_PROVIDER", "env")
        .env("BACKFABRIC_NODE_KEY", key)
        // secretspec asks for a reason when it detects a coding agent.
        .env("SECRETSPEC_REASON", "backfabric tests")
        .output()
        .unwrap()
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
    let expected: iroh::SecretKey = key.parse().unwrap();

    let output = id(&key);

    assert!(output.status.success(), "{}", stderr(&output));
    let printed = String::from_utf8(output.stdout).unwrap();
    assert_eq!(printed.trim(), expected.public().to_string());
}

#[test]
fn id_refuses_key_that_is_not_64_hex_characters() {
    for key in ["abc", &"zz".repeat(32), &"11".repeat(33)] {
        let output = id(key);

        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("must be 64 hex characters"),
            "{}",
            stderr(&output)
        );
    }
}
