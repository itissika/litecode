pub mod action;
pub mod engine;
pub mod evaluate;
pub mod floor;
pub mod grants;
pub mod matchers;
pub mod messages;
pub mod policy;
pub mod presets;
pub mod sensitive;
pub mod sinks;

pub use action::PermissionAction;
pub use engine::{PermissionEngine, PermissionView};
pub use evaluate::{EvalResult, evaluate};
pub use grants::{
    AskOutcome, PermissionSink, blocking_wait_oneshot, blocking_wait_oneshot_cancellable,
    check_runtime_grant, clear_runtime_grants, clear_runtime_grants_for, grant_runtime,
};
pub use matchers::{ArgMatcher, MatchContext, matches};
pub use messages::permission_denied_message;
pub use policy::{BindingPathMode, DEFAULT_RULE_ID, PolicyRule, ToolPolicy};
pub use presets::{apply_preset_to_tools, binding_for_tool};
pub use sensitive::is_sensitive_system_path;
pub use sinks::{
    CancellingPermissionSink, DenyPermissionSink, RecordingPermissionSink, deny_permission_sink,
};
