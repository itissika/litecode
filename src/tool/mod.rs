pub mod agent_bindings;
pub mod authorize;
pub mod availability;
pub mod catalog;
pub mod coords;
pub mod executor;
pub mod output;
pub mod pipeline;
pub mod registry;
pub mod schema_validate;
pub mod signal;
pub mod snippet;
pub mod trait_;
pub mod write_lock;

pub use authorize::{AuthResult, authorize};
pub use coords::{
    FILE_LINE_PREFIX_HINT, attach_offset_footer, format_file_line, format_file_window_footer,
    format_line_label, format_line_list, format_offset_done, format_offset_more, format_path_lines,
};
pub use pipeline::ToolPipeline;
pub use registry::build_tool_list;
pub use schema_validate::{
    check_tool_input, expected_type, invalid_input_for, missing_parameter, must_be,
    must_be_nonempty_string, must_be_one_of, parse_tool_arguments, require_nonempty_string,
    require_nonempty_string_trimmed, require_string, require_string_value, schema_validate,
    unknown_top_level_properties,
};
pub use snippet::{SnippetSection, format_snippet_sections};
pub use trait_::Tool;
pub use write_lock::{ResourceKey, WorkspaceWriteLock};
