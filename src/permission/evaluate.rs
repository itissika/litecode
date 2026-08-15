use serde_json::Value;

use super::action::PermissionAction;
use super::matchers::{ArgMatcher, MatchContext, matches};
use super::policy::{DEFAULT_RULE_ID, PolicyRule, ToolPolicy};

pub struct EvalResult {
    pub rule_id: String,
    pub action: PermissionAction,
}

pub fn evaluate(policy: &ToolPolicy, args: &Value, ctx: &MatchContext<'_>) -> EvalResult {
    for rule in &policy.rules {
        if matches(&rule.when, args, ctx) {
            return EvalResult {
                rule_id: rule.id.clone(),
                action: rule.action,
            };
        }
    }
    EvalResult {
        rule_id: policy.default_id.clone(),
        action: policy.default,
    }
}

pub fn evaluate_rules(
    rules: &[PolicyRule],
    default_id: &str,
    default: PermissionAction,
    args: &Value,
    ctx: &MatchContext<'_>,
) -> EvalResult {
    for rule in rules {
        if matches(&rule.when, args, ctx) {
            return EvalResult {
                rule_id: rule.id.clone(),
                action: rule.action,
            };
        }
    }
    EvalResult {
        rule_id: default_id.to_string(),
        action: default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::policy::ToolPolicy;

    #[test]
    fn first_matching_rule_wins() {
        let policy = ToolPolicy {
            default: PermissionAction::Deny,
            default_id: DEFAULT_RULE_ID.into(),
            rules: vec![
                PolicyRule {
                    id: "readonly".into(),
                    when: ArgMatcher::BashReadonlyCommand,
                    action: PermissionAction::Allow,
                },
                PolicyRule {
                    id: "other".into(),
                    when: ArgMatcher::Any,
                    action: PermissionAction::Ask,
                },
            ],
        };
        let ctx = MatchContext {
            workspace_root: std::path::Path::new("/tmp"),
            path_mode: crate::permission::policy::BindingPathMode::Unrestricted,
        };
        let r = evaluate(&policy, &serde_json::json!({"command": "ls"}), &ctx);
        assert_eq!(r.rule_id, "readonly");
        assert_eq!(r.action, PermissionAction::Allow);
    }
}
