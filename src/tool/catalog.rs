//! Agent bind helpers (legacy module path).

pub use super::agent_bindings::{
    available_id_set, merge_agent_tool_bindings, normalize_agent_profile,
    strip_subagent_series_bindings,
};
pub use super::availability::{
    agent_tool_enabled, available_tools, is_available, should_include_in_llm_list,
};
