//! Integration tests — multi-request scenarios, SSE streaming, TTL.
//!
//! Run with: `cargo nextest run --test integration`

#[path = "../common/mod.rs"]
mod common;

mod body_limit;
mod cassette_infrastructure;
mod doppel;
mod timeout;
mod ttl;
