//! Core types, hashing, SQLite-backed cache, and provider definitions for `lcp`.
//!
//! This crate contains the data model ([`types`]), the BLAKE3-based cache key
//! derivation ([`hash`]), the SQLite cache store ([`cache`]), and the provider
//! enum ([`provider`]). It has no HTTP dependencies and can be used independently
//! of `lcp-server`.
//!
//! # Data model
//!
//! An [`Exchange`] captures a complete request/response pair: the [`RequestRecord`],
//! HTTP status, content type, and a sequence of [`ResponseChunk`]s (one per SSE
//! event for streaming responses, one chunk for JSON responses).
//!
//! [`Cache`] stores exchanges keyed by a BLAKE3 digest computed over the
//! normalized request body (see [`cache_key`]). Key normalization strips
//! transport-only and provider-specific attribution fields so that logically
//! identical requests always hit the same cache entry.
#![warn(missing_docs)]

/// SQLite-backed cache store.
pub mod cache;
/// BLAKE3 cache key derivation and request normalization.
pub mod hash;
/// Supported upstream LLM provider enum.
pub mod provider;
/// Core data model: exchanges, chunks, and cache entry types.
pub mod types;

pub use cache::Cache;
pub use hash::{cache_key, cache_key_and_model};
pub use provider::Provider;
pub use types::{CacheEntry, Exchange, FullEntry, RequestRecord, ResponseChunk};
