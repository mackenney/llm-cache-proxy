//! End-to-end tests against real providers.
//!
//! Gated by `--features test-e2e`. Requires API keys.
//!
//! Run with: `cargo nextest run --test e2e --features test-e2e`

#[path = "../common/mod.rs"]
mod common;

mod cli;

mod sse_fields;
