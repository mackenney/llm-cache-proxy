//! CLI-1: Config flag / env-var precedence tests.
//!
//! These tests spawn the real `lcp` binary with `--print-config` and verify
//! that the effective configuration reflects the correct precedence:
//!   CLI flag > env var > config file > built-in default
//!
//! No network or running server is required.

use assert_cmd::Command;
use std::io::Write;
use tempfile::NamedTempFile;

fn lcp() -> Command {
    let mut cmd =
        Command::cargo_bin("lcp").expect("lcp binary not found; run `cargo build --bin lcp` first");
    // Isolate from the real user config; individual tests override via --config or LCP_CONFIG.
    cmd.env("LCP_CONFIG", "/dev/null");
    cmd
}

fn write_config(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(content.as_bytes()).expect("write");
    f
}

// Built-in defaults

#[test]
fn default_port_is_9001() {
    let out = lcp().arg("--print-config").output().unwrap();
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("port = 9001"), "stdout:\n{stdout}");
}

#[test]
fn default_host_is_loopback() {
    let out = lcp().arg("--print-config").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("host = \"127.0.0.1\""), "stdout:\n{stdout}");
}

#[test]
fn default_ttl_is_zero() {
    let out = lcp().arg("--print-config").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ttl = 0"), "stdout:\n{stdout}");
}

#[test]
fn default_timeout_is_300() {
    let out = lcp().arg("--print-config").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("timeout = 300"), "stdout:\n{stdout}");
}

#[test]
fn default_body_limit_is_100mb() {
    let out = lcp().arg("--print-config").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("body_limit = 104857600"),
        "stdout:\n{stdout}"
    );
}

// Upstream options without a value are commented out

#[test]
fn upstream_options_commented_when_unset() {
    let out = lcp().arg("--print-config").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("# anthropic_upstream"), "stdout:\n{stdout}");
    assert!(stdout.contains("# openai_upstream"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("# openrouter_upstream"),
        "stdout:\n{stdout}"
    );
    assert!(stdout.contains("# gemini_upstream"), "stdout:\n{stdout}");
}

// env var overrides built-in default

#[test]
fn lcp_port_env_var_overrides_default() {
    let out = lcp()
        .env("LCP_PORT", "9002")
        .arg("--print-config")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("port = 9002"), "stdout:\n{stdout}");
}

#[test]
fn lcp_db_env_var_sets_db_path() {
    let out = lcp()
        .env("LCP_DB", "/tmp/env-test.db")
        .arg("--print-config")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("db = \"/tmp/env-test.db\""),
        "stdout:\n{stdout}"
    );
}

// CLI flag overrides env var

#[test]
fn cli_port_flag_beats_env_var() {
    let out = lcp()
        .env("LCP_PORT", "9002")
        .args(["--port", "9003", "--print-config"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("port = 9003"), "stdout:\n{stdout}");
}

#[test]
fn cli_db_flag_beats_env_var() {
    let out = lcp()
        .env("LCP_DB", "/tmp/env.db")
        .args(["--db", "/tmp/flag.db", "--print-config"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("db = \"/tmp/flag.db\""),
        "stdout:\n{stdout}"
    );
}

// Config file overrides built-in default

#[test]
fn config_file_port_overrides_default() {
    let f = write_config("port = 9004\n");
    let out = lcp()
        .args(["--config", f.path().to_str().unwrap(), "--print-config"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("port = 9004"), "stdout:\n{stdout}");
}

#[test]
fn config_file_db_overrides_default() {
    let f = write_config("db = \"/tmp/file.db\"\n");
    let out = lcp()
        .args(["--config", f.path().to_str().unwrap(), "--print-config"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("db = \"/tmp/file.db\""),
        "stdout:\n{stdout}"
    );
}

#[test]
fn config_file_sets_upstream() {
    let f = write_config("anthropic_upstream = \"http://localhost:8080\"\n");
    let out = lcp()
        .args(["--config", f.path().to_str().unwrap(), "--print-config"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("anthropic_upstream = \"http://localhost:8080\""),
        "stdout:\n{stdout}"
    );
}

// env var beats config file

#[test]
fn env_var_beats_config_file_port() {
    let f = write_config("port = 9004\n");
    let out = lcp()
        .env("LCP_PORT", "9002")
        .args(["--config", f.path().to_str().unwrap(), "--print-config"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("port = 9002"), "stdout:\n{stdout}");
}

// CLI flag beats config file

#[test]
fn cli_flag_beats_config_file_port() {
    let f = write_config("port = 9004\n");
    let out = lcp()
        .args([
            "--config",
            f.path().to_str().unwrap(),
            "--port",
            "9003",
            "--print-config",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("port = 9003"), "stdout:\n{stdout}");
}

// Malformed config file emits warning and is ignored (does not crash)

#[test]
fn malformed_config_file_is_ignored_with_warning() {
    let f = write_config("this is not valid toml !!!\n");
    let out = lcp()
        .args(["--config", f.path().to_str().unwrap(), "--print-config"])
        .output()
        .unwrap();
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning"),
        "expected warning in stderr:\n{stderr}"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Falls back to built-in defaults
    assert!(stdout.contains("port = 9001"), "stdout:\n{stdout}");
}

// Upstream set via CLI flag is shown without comment

#[test]
fn cli_upstream_flag_shown_uncommented() {
    let out = lcp()
        .args([
            "--anthropic-upstream",
            "http://localhost:8080",
            "--print-config",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("anthropic_upstream = \"http://localhost:8080\""),
        "stdout:\n{stdout}"
    );
}

// LCP_CONFIG env var points to a config file

#[test]
fn lcp_config_env_var_loads_file() {
    let f = write_config("port = 9005\n");
    let out = lcp()
        .env("LCP_CONFIG", f.path().to_str().unwrap())
        .arg("--print-config")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("port = 9005"), "stdout:\n{stdout}");
}

// --config CLI flag beats LCP_CONFIG env var (CLI > env precedence for config path)

#[test]
fn cli_config_flag_beats_lcp_config_env_var() {
    let env_cfg = write_config("port = 9010\n");
    let flag_cfg = write_config("port = 9011\n");
    let out = lcp()
        .env("LCP_CONFIG", env_cfg.path().to_str().unwrap())
        .args([
            "--config",
            flag_cfg.path().to_str().unwrap(),
            "--print-config",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("port = 9011"),
        "--config flag must beat LCP_CONFIG env var; stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("port = 9010"),
        "LCP_CONFIG env var must not win over --config flag; stdout:\n{stdout}"
    );
}
