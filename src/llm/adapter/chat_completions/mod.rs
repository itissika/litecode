//! Shared Chat Completions codec — Items ↔ messages JSON / SSE synthesis.
//!
//! Vendor adapters own HTTP headers, hosts, and extra body fields. Kernel still
//! sees authority Items / `ResponseStreamEvent` only.

mod catalog;
mod decode;
mod encode;
mod http;
mod stream;
mod usage;

pub(crate) use catalog::{chat_post_url, models_get_url, normalize_endpoint, parse_model_catalog};
pub(super) use encode::{ChatEncodeOpts, encode_chat_body};
pub(super) use http::{complete_from_response, stream_from_response};

#[cfg(test)]
mod tests;
