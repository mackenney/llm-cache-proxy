//! Spec invariant tests — behavioral contracts from SPEC.md.
//!
//! Run with: `cargo nextest run --test spec`

#[path = "../common/mod.rs"]
mod common;

mod cache_hit;
mod cache_miss;
mod model_extraction;
