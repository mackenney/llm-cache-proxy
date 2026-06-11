//! Spec invariant tests — behavioral contracts from SPEC.md.
//!
//! Run with: `cargo nextest run --test spec`

#[path = "../common/mod.rs"]
mod common;

mod admin;
mod body_limit;
mod bypass;
mod cache_hit;
mod cache_miss;
mod compression;
mod extensions;
mod forwarding;
mod model_extraction;
mod normalization;
mod routing;
mod sse_restore_streaming;
mod sse_terminal_ordering;
mod tracing;
