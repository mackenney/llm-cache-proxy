//! HTTP proxy server with caching and extension pipeline for `lcp`.
//!
//! This crate provides the [`serve`] entry point, [`ServerConfig`],
//! the three-phase [`ExtensionPipeline`], and the built-in [`DoppelExt`]
//! secret-swapping extension.
//!
//! # Extension pipeline
//!
//! Extensions implement the [`Extension`] trait and are composed into an
//! [`ExtensionPipeline`]. Three phases fire per proxied request:
//!
//! | Phase | When | Purpose |
//! |-------|------|---------|
//! | 1 — [`on_request_body`] | every request, before cache key | normalize/inspect the body |
//! | 2 — [`on_upstream_body`] | cache miss only, before forwarding | transform the wire body |
//! | 3 — [`on_response_stream`] | cache miss only, after upstream responds | wrap the response stream |
//!
//! [`on_request_body`]: Extension::on_request_body
//! [`on_upstream_body`]: Extension::on_upstream_body
//! [`on_response_stream`]: Extension::on_response_stream
#![warn(missing_docs)]

/// Built-in extensions (`DoppelExt` and SSE restore stream).
pub mod ext;
/// Extension trait and pipeline types.
pub mod extensions;
/// Axum proxy handler and application state.
pub mod proxy;
/// Axum router construction.
pub mod router;
/// Server configuration and startup.
pub mod server;
/// Admin and stats HTTP handlers.
pub mod stats;

pub use ext::{DoppelExt, DoppelExtLoadError};
pub use extensions::{
    Extension, ExtensionPipeline, ProxyCtx, ResponseStream, SensitiveState, SensitiveStateBuilder,
};
pub use server::{ServerConfig, build_upstream_client, serve};
