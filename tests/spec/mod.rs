//! Spec invariant tests — behavioral contracts from SPEC.md.
//!
//! Run with: `cargo nextest run --test spec`

#[path = "../common/mod.rs"]
mod common;

mod admin;
mod bypass;
mod cache_hit;
mod cache_miss;
mod compression;
mod forwarding;
mod model_extraction;
mod normalization;
mod routing;
mod tracing;
