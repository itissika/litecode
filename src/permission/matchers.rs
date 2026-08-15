use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tools::bash_safety::is_readonly_command;
use crate::workspace::raw_path_outside_workspace;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArgMatcher {
    Any,
    ArgEquals { name: String, value: String },
    ArgGlob { name: String, pattern: String },
    PathOutsideWorkspace { name: String },
    BashReadonlyCommand,
    AllOf { matchers: Vec<ArgMatcher> },
    AnyOf { matchers: Vec<ArgMatcher> },
}

pub struct MatchContext<'a> {
    pub workspace_root: &'a Path,
    pub path_mode: crate::permission::policy::BindingPathMode,
}

pub fn matches(matcher: &ArgMatcher, args: &Value, ctx: &MatchContext<'_>) -> bool {
    match matcher {
        ArgMatcher::Any => true,
        ArgMatcher::ArgEquals { name, value } => args
            .get(name)
            .and_then(Value::as_str)
            .is_some_and(|v| v == value),
        ArgMatcher::ArgGlob { name, pattern } => args
            .get(name)
            .and_then(Value::as_str)
            .is_some_and(|v| glob_match(pattern, v)),
        ArgMatcher::PathOutsideWorkspace { name } => args
            .get(name)
            .and_then(Value::as_str)
            .is_some_and(|path| raw_path_outside_workspace(ctx.workspace_root, path)),
        ArgMatcher::BashReadonlyCommand => args
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(is_readonly_command),
        ArgMatcher::AllOf { matchers } => matchers.iter().all(|m| matches(m, args, ctx)),
        ArgMatcher::AnyOf { matchers } => matchers.iter().any(|m| matches(m, args, ctx)),
    }
}

fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == value;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 2 {
        let prefix = parts[0];
        let suffix = parts[1];
        return value.starts_with(prefix) && value.ends_with(suffix);
    }
    pattern == value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_star_matches_all() {
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn arg_equals_matches() {
        let args = serde_json::json!({"command": "ls"});
        let m = ArgMatcher::ArgEquals {
            name: "command".into(),
            value: "ls".into(),
        };
        let ctx = MatchContext {
            workspace_root: Path::new("/tmp"),
            path_mode: crate::permission::policy::BindingPathMode::Unrestricted,
        };
        assert!(matches(&m, &args, &ctx));
    }
}
