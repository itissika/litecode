// Excerpt: async-openai 0.41.1 — src/types/shared/response_usage.rs
// (Authority for Responses `usage` on completed events)

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct InputTokenDetails {
    pub cached_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct OutputTokenDetails {
    pub reasoning_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ResponseUsage {
    pub input_tokens: u32,
    pub input_tokens_details: InputTokenDetails,
    pub output_tokens: u32,
    pub output_tokens_details: OutputTokenDetails,
    pub total_tokens: u32,
}
