//! Custom-suite binding migration. A suite is one capability with one binding;
//! legacy profiles may still carry one row per member. Collapsing is idempotent.

use std::collections::{BTreeMap, HashMap};

use crate::config::schema::{AgentToolBinding, CustomToolDefinition, ToolPreset};

pub fn suite_id(name: &str) -> String {
    format!("suite:{name}")
}

fn groups(defs: &[CustomToolDefinition]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for def in defs {
        if let Some(suite) = def.suite.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            out.entry(suite.to_string())
                .or_default()
                .push(def.name.clone());
        }
    }
    out
}

fn preset_rank(preset: ToolPreset) -> u8 {
    match preset {
        ToolPreset::Safe => 2,
        ToolPreset::All => 1,
    }
}

/// Merge legacy member rows into the suite row, then remove the member rows.
/// Returns human-readable conflicts for the caller to log; it never silently
/// drops a non-empty binding.
pub fn collapse_custom_suite_bindings(
    tools: &mut HashMap<String, AgentToolBinding>,
    defs: &[CustomToolDefinition],
) -> Vec<String> {
    let mut notes = Vec::new();
    for (suite, members) in groups(defs) {
        let id = suite_id(&suite);
        let existing = tools.remove(&id);
        let legacy: Vec<(String, AgentToolBinding)> = members
            .iter()
            .filter_map(|member| tools.remove(member).map(|binding| (member.clone(), binding)))
            .collect();
        if existing.is_none() && legacy.is_empty() {
            continue;
        }

        let enabled = existing
            .as_ref()
            .is_some_and(|binding| binding.enabled)
            || legacy.iter().any(|(_, binding)| binding.enabled);
        let preset = existing
            .as_ref()
            .and_then(|binding| binding.last_applied_preset)
            .or_else(|| {
                legacy
                    .iter()
                    .filter_map(|(_, binding)| binding.last_applied_preset)
                    .max_by_key(|preset| preset_rank(*preset))
            });
        let (policy, path_mode) = existing
            .as_ref()
            .map(|binding| (binding.policy.clone(), binding.path_mode))
            .or_else(|| {
                legacy
                    .iter()
                    .next()
                    .map(|(_, binding)| (binding.policy.clone(), binding.path_mode))
            })
            .unwrap_or_else(|| {
                preset
                    .map(|preset| crate::permission::binding_for_tool(&id, preset))
                    .unwrap_or_else(|| {
                        (crate::permission::ToolPolicy::default(), Default::default())
                    })
            });
        let allowed_tools = existing
            .as_ref()
            .and_then(|binding| binding.allowed_tools.clone())
            .or_else(|| {
                legacy
                    .iter()
                    .find_map(|(_, binding)| binding.allowed_tools.clone())
            });
        if existing.is_some() && !legacy.is_empty() {
            notes.push(format!(
                "custom suite '{suite}' had both a suite row and {} legacy member row(s); merged",
                legacy.len()
            ));
        }
        if legacy.len() > 1 {
            let first_policy = legacy.first().map(|(_, binding)| &binding.policy);
            if legacy.iter().any(|(_, binding)| &binding.policy != first_policy.unwrap()) {
                notes.push(format!(
                    "custom suite '{suite}' had conflicting legacy policies; kept the first row"
                ));
            }
        }
        // Keep the first legacy row as the permission template. The suite id
        // has no rule entry, so `binding_for_tool` would replace a real member
        // policy with the generic Ask default.
        tools.insert(
            id,
            AgentToolBinding {
                enabled,
                policy,
                path_mode,
                last_applied_preset: preset,
                allowed_tools,
            },
        );
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn custom(name: &str, suite: Option<&str>) -> CustomToolDefinition {
        CustomToolDefinition {
            name: name.into(),
            description: String::new(),
            schema: crate::config::schema::ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 30,
            rules: Vec::new(),
            surface: Default::default(),
            suite: suite.map(str::to_string),
            suite_label: None,
            data_schema: None,
        }
    }

    fn binding(enabled: bool, preset: Option<ToolPreset>) -> AgentToolBinding {
        AgentToolBinding {
            enabled,
            policy: crate::permission::ToolPolicy::default(),
            path_mode: Default::default(),
            last_applied_preset: preset,
            allowed_tools: None,
        }
    }

    #[test]
    fn legacy_member_rows_become_one_suite_row() {
        let defs = vec![custom("a", Some("deck")), custom("b", Some("deck"))];
        let mut tools = HashMap::from([
            ("a".into(), binding(true, Some(ToolPreset::Safe))),
            ("b".into(), binding(false, Some(ToolPreset::All))),
        ]);
        let notes = collapse_custom_suite_bindings(&mut tools, &defs);
        assert!(notes.is_empty());
        assert!(!tools.contains_key("a"));
        assert!(!tools.contains_key("b"));
        let suite = tools.get("suite:deck").expect("suite row");
        assert!(suite.enabled);
        assert_eq!(suite.last_applied_preset, Some(ToolPreset::Safe));
    }

    #[test]
    fn legacy_preset_keeps_the_member_policy_template() {
        let defs = vec![custom("a", Some("deck")), custom("b", Some("deck"))];
        let mut first = binding(true, Some(ToolPreset::Safe));
        first.policy = crate::permission::ToolPolicy::allow_all();
        first.path_mode = crate::permission::BindingPathMode::Unrestricted;
        let mut tools = HashMap::from([
            ("a".into(), first),
            ("b".into(), binding(false, Some(ToolPreset::All))),
        ]);

        collapse_custom_suite_bindings(&mut tools, &defs);

        let suite = tools.get("suite:deck").expect("suite row");
        assert_eq!(suite.last_applied_preset, Some(ToolPreset::Safe));
        assert_eq!(suite.policy, crate::permission::ToolPolicy::allow_all());
        assert_eq!(
            suite.path_mode,
            crate::permission::BindingPathMode::Unrestricted
        );
    }

    #[test]
    fn collapse_is_idempotent_and_does_not_touch_single_tools() {
        let defs = vec![custom("a", Some("deck")), custom("b", Some("deck"))];
        let mut tools = HashMap::from([
            ("a".into(), binding(true, None)),
            ("single".into(), binding(false, None)),
        ]);
        collapse_custom_suite_bindings(&mut tools, &defs);
        let once = tools.get("suite:deck").unwrap().clone();
        collapse_custom_suite_bindings(&mut tools, &defs);
        assert_eq!(tools.get("suite:deck"), Some(&once));
        assert!(tools.contains_key("single"));
    }
}
