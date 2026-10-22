//! Agent data-shape authority (Phase 0 freeze).
//!
//! **Hard rule:** conversation atoms and stream events used as kernel truth MUST be
//! types from [`async_openai::types::responses`]. Do not define parallel
//! `Message` / `ContentBlock` / homemade `StreamEvent` dialects here or elsewhere.
//!
//! This module only re-exports. It does not wrap, subset, or rename serde shapes.

/// OpenAI Responses API types — sole Rust authority for agent transcript & stream.
pub mod responses {
    pub use async_openai::types::responses::*;
}

#[cfg(test)]
mod tests {
    

}
