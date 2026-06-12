//! Integration tests — multi-request scenarios, SSE streaming, TTL.
//!
//! Run with: `cargo nextest run --test integration`

#[path = "../common/mod.rs"]
mod common;

mod body_limit;
mod cassette_infrastructure;
mod cassettes;
mod concurrent;
mod doppel;
mod errors;
mod sse_detection;
mod timeout;
mod ttl;
