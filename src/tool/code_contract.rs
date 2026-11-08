//! Generated JS/TS tool contract and the runtime bridge-facing schema view.
//!
//! The contract source is `sdk/src/api.d.ts`. Generated JSON artifacts are
//! committed so `cargo build` does not require Node. This module never derives
//! public code types from Rust tool schemas.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::tool::Tool;
use crate::tool::dynamic_contract::{self, DynamicTool};
use crate::tool::schema_validate::schema_validate;
use crate::types::{ToolCallResult, ToolSignalLevel};

const CONTRACT_JSON: &str = include_str!("../../sdk/generated/tool-contract.json");
const FRAGMENTS_JSON: &str = include_str!("../../sdk/generated/sdk-fragments.json");

#[derive(Debug, Clone, Deserialize)]
pub struct ToolSchemaContract {
    pub path: Vec<String>,
    pub suite: String,
    pub tool: String,
    pub doc: String,
    pub signature: String,
    pub input_type: String,
    pub output_type: String,
    pub type_deps: Vec<String>,
    pub input_schema: Value,
    pub output_schema: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolContract {
    pub version: u32,
    pub sdk_hash: String,
    pub tools: BTreeMap<String, ToolSchemaContract>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SdkToolFragment {
    pub path: Vec<String>,
    pub suite: String,
    pub doc: String,
    pub signature: String,
    pub type_deps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SdkFragments {
    pub version: u32,
    pub sdk_hash: String,
    pub prose: String,
    pub declarations: String,
    pub libs: String,
    pub types: BTreeMap<String, String>,
    pub tools: BTreeMap<String, SdkToolFragment>,
}

#[derive(Debug, Clone)]
pub struct CodeToolContract {
    pub input_schema: Value,
    pub output_schema: Value,
}

/// One concrete code surface: assembled SDK text plus per-tool schema checks.
#[derive(Debug, Clone)]
pub struct CodeSurface {
    pub sdk: String,
    pub sdk_hash: String,
    /// Runtime tool name -> generated input/output contract.
    pub tools: BTreeMap<String, CodeToolContract>,
    /// JavaScript binding path (`suite.member`) -> runtime tool name.
    pub bindings: BTreeMap<String, String>,
}

impl CodeSurface {
    pub fn validate_input(&self, tool: &str, input: &Value) -> Result<(), String> {
        let contract = self
            .tools
            .get(tool)
            .ok_or_else(|| format!("tool '{tool}' has no code contract"))?;
        schema_validate(&contract.input_schema, input)
    }

    pub fn validate_output(&self, tool: &str, output: &Value) -> Result<(), String> {
        let contract = self
            .tools
            .get(tool)
            .ok_or_else(|| format!("tool '{tool}' has no code contract"))?;
        schema_validate(&contract.output_schema, output)
    }

    /// The run_code output gate for one nested tool result. Returns the
    /// validation error when the result is a success whose program value does
    /// not match the tool's output schema; the bridge turns that into
    /// 'invalid_output'. Failures and non-success levels pass through unchecked.
    /// The output drift test calls this same method.
    pub fn output_violation(&self, tool: &str, result: &ToolCallResult) -> Option<String> {
        let success = result.error.is_none()
            && matches!(result.level, ToolSignalLevel::Ok | ToolSignalLevel::Warning);
        if !success {
            return None;
        }
        let program_value = crate::tool::outcome::program_result(result).ok()?;
        self.validate_output(tool, &program_value).err()
    }
}

/// One output drift case for one built-in code tool. Each module builds its own
/// cases next to its output type, from the production record types, so the
/// drift test checks what the tool really returns.
#[cfg(test)]
pub(crate) struct DriftCase {
    pub tool: &'static str,
    /// Every optional property present.
    pub all_optional_present: crate::types::ToolCallResult,
    /// Every optional property absent.
    pub all_optional_absent: crate::types::ToolCallResult,
}

struct Builtins {
    contract: ToolContract,
    fragments: SdkFragments,
}

fn builtins() -> &'static Builtins {
    static BUILTINS: OnceLock<Builtins> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        let contract: ToolContract =
            serde_json::from_str(CONTRACT_JSON).expect("parse generated tool contract");
        let fragments: SdkFragments =
            serde_json::from_str(FRAGMENTS_JSON).expect("parse generated SDK fragments");
        validate_builtins(&contract, &fragments).expect("validate generated tool contract");
        Builtins {
            contract,
            fragments,
        }
    })
}

fn validate_builtins(contract: &ToolContract, fragments: &SdkFragments) -> Result<(), String> {
    if contract.version != 1 || fragments.version != 1 {
        return Err("unsupported generated tool-contract version".into());
    }
    if contract.sdk_hash != fragments.sdk_hash {
        return Err(format!(
            "generated contract hash mismatch: {} vs {}",
            contract.sdk_hash, fragments.sdk_hash
        ));
    }
    if contract.tools.keys().collect::<Vec<_>>() != fragments.tools.keys().collect::<Vec<_>>() {
        return Err("generated contract and SDK fragment tool sets differ".into());
    }
    for (name, tool) in &contract.tools {
        for dep in &tool.type_deps {
            if !fragments.types.contains_key(dep) {
                return Err(format!("tool '{name}' references missing SDK type '{dep}'"));
            }
        }
        if tool.input_schema.is_null() || tool.output_schema.is_null() {
            return Err(format!("tool '{name}' has an empty input/output schema"));
        }
    }
    Ok(())
}

/// The JS SDK path of one built-in code tool (`read` -> `file.read`). This is
/// the same table `bindings` is built from, so a message that names the path a
/// caller should have used cannot drift from the SDK the caller actually sees.
pub fn builtin_sdk_path(name: &str) -> Option<String> {
    builtins()
        .fragments
        .tools
        .get(name)
        .map(|fragment| fragment.path.join("."))
}

/// Input schema for one built-in code tool, derived from the JS/TS contract.
pub fn builtin_input_schema(name: &str) -> Option<Value> {
    builtins()
        .contract
        .tools
        .get(name)
        .map(|tool| tool.input_schema.clone())
}

/// Build the concrete contract for the tools bound into one `run_code` call.
pub fn build_surface(tools: &[Arc<dyn Tool>]) -> Result<CodeSurface, String> {
    let builtins = builtins();
    let mut fragments = builtins.fragments.clone();
    let mut contracts = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    let mut selected = BTreeSet::new();
    let mut reserved_types: BTreeSet<String> = fragments.types.keys().cloned().collect();

    for tool in tools {
        let name = tool.name().to_string();
        if let Some(spec) = tool.dynamic_code_spec() {
            let dynamic: DynamicTool =
                dynamic_contract::build_dynamic_tool(&name, spec, &reserved_types)?;
            for (type_name, text) in dynamic.types {
                if fragments.types.insert(type_name.clone(), text).is_some() {
                    return Err(format!(
                        "dynamic tool '{name}' produced a duplicate SDK type '{type_name}'"
                    ));
                }
                reserved_types.insert(type_name);
            }
            let sdk_path = dynamic.fragment.path.join(".");
            if bindings.insert(sdk_path.clone(), name.clone()).is_some() {
                return Err(format!("duplicate SDK binding path '{sdk_path}'"));
            }
            fragments.tools.insert(name.clone(), dynamic.fragment);
            contracts.insert(
                name.clone(),
                CodeToolContract {
                    input_schema: dynamic.input_schema,
                    output_schema: dynamic.output_schema,
                },
            );
            selected.insert(name);
            continue;
        }

        let contract = builtins
            .contract
            .tools
            .get(&name)
            .ok_or_else(|| format!("code tool '{name}' has no built-in JS SDK contract"))?;
        let fragment = builtins
            .fragments
            .tools
            .get(&name)
            .ok_or_else(|| format!("code tool '{name}' has no built-in SDK fragment"))?;
        let sdk_path = fragment.path.join(".");
        if bindings.insert(sdk_path.clone(), name.clone()).is_some() {
            return Err(format!("duplicate SDK binding path '{sdk_path}'"));
        }
        fragments.tools.insert(name.clone(), fragment.clone());
        contracts.insert(
            name.clone(),
            CodeToolContract {
                input_schema: contract.input_schema.clone(),
                output_schema: contract.output_schema.clone(),
            },
        );
        selected.insert(name);
    }

    let sdk = crate::tool::code_sdk::assemble(&fragments, &selected)?;
    let sdk_hash = format!("{:x}", Sha256::digest(sdk.as_bytes()));
    Ok(CodeSurface {
        sdk,
        sdk_hash,
        tools: contracts,
        bindings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_contract_loads() {
        let builtins = builtins();
        assert!(!builtins.contract.tools.is_empty());
        assert_eq!(builtins.contract.sdk_hash, builtins.fragments.sdk_hash);
    }

    #[test]
    fn full_builtin_sdk_matches_generated_fixture() {
        let tools = crate::tool::registry::builtin_code_surface_tools();
        let surface = build_surface(&tools).expect("assemble full builtin SDK");
        let expected = include_str!("../../sdk/generated/all-code-sdk.txt");
        assert_eq!(surface.sdk, expected);
    }

    #[test]
    fn builtin_bindings_and_noarg_contracts_are_consistent() {
        let tools = crate::tool::registry::builtin_code_surface_tools();
        let surface = build_surface(&tools).expect("assemble full builtin SDK");
        assert_eq!(
            surface.bindings.get("file.read").map(String::as_str),
            Some("read")
        );
        assert_eq!(
            surface.bindings.get("memory.list").map(String::as_str),
            Some("memory.list")
        );
        for path in ["browser.list", "memory.list"] {
            let tool = &surface.tools[surface.bindings[path].as_str()];
            assert_eq!(tool.input_schema["type"], "object", "{path}");
            assert_eq!(tool.input_schema["additionalProperties"], false, "{path}");
            assert_eq!(tool.output_schema["required"][0], "ok", "{path}");
        }
    }

    #[test]
    fn generated_schemas_are_inline_validator_subset() {
        fn check(value: &Value, path: &str) {
            if let Some(object) = value.as_object() {
                assert!(!object.contains_key("$ref"), "{path}: $ref must be inlined");
                assert!(
                    !object.contains_key("allOf"),
                    "{path}: allOf is unsupported"
                );
                for (key, child) in object {
                    check(child, &format!("{path}.{key}"));
                }
            } else if let Some(items) = value.as_array() {
                for (index, child) in items.iter().enumerate() {
                    check(child, &format!("{path}[{index}]"));
                }
            }
        }
        let builtins = builtins();
        for (name, tool) in &builtins.contract.tools {
            check(&tool.input_schema, &format!("{name}.input"));
            check(&tool.output_schema, &format!("{name}.output"));
        }
    }

    #[test]
    fn run_contract_allows_explicit_background_start() {
        let schema = builtin_input_schema("run").expect("run schema");
        assert_eq!(schema["properties"]["run_in_background"]["type"], "boolean");
        schema_validate(
            &schema,
            &serde_json::json!({"command": "sleep 30", "run_in_background": true}),
        )
        .expect("background run input");
    }

    #[test]
    fn subset_assembly_contains_only_bound_tools_and_types() {
        let read: Arc<dyn Tool> = Arc::new(crate::tools::read::ReadTool::default());
        let first = build_surface(&[read.clone()]).expect("read surface");
        let second = build_surface(&[read]).expect("read surface twice");
        assert_eq!(first.sdk, second.sdk);
        assert!(first.sdk.contains("read(args: ReadInput)"));
        assert!(!first.sdk.contains("GrepInput"));
        assert!(!first.sdk.contains("grep(args:"));
    }

    struct DynamicTool {
        name: &'static str,
    }

    impl Tool for DynamicTool {
        fn name(&self) -> &str {
            self.name
        }

        fn surface(&self) -> crate::tool::ToolSurface {
            crate::tool::ToolSurface::Code
        }

        fn dynamic_code_spec(&self) -> Option<crate::tool::DynamicCodeSpec> {
            Some(crate::tool::DynamicCodeSpec {
                description: format!("dynamic {}", self.name),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
                output_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"]
                }),
            })
        }
    }

    #[test]
    fn dynamic_suite_members_are_nested_and_type_names_are_unique() {
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(DynamicTool { name: "deck.only" }),
            Arc::new(DynamicTool { name: "deck.also" }),
        ];
        let surface = build_surface(&tools).expect("dynamic surface");
        assert!(surface.sdk.contains("deck: {"), "{}", surface.sdk);
        assert!(surface.sdk.contains("only(args: CustomDeckOnlyInput)"));
        assert!(surface.sdk.contains("also(args: CustomDeckAlsoInput)"));
        assert!(surface.tools.contains_key("deck.only"));
        assert!(surface.tools.contains_key("deck.also"));
    }

    #[test]
    fn builtin_input_schemas_have_no_hard_numeric_bounds() {
        fn check(tool: &str, value: &Value, path: &str) {
            match value {
                Value::Object(object) => {
                    for key in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
                        assert!(
                            !object.contains_key(key),
                            "{tool} input schema sets {path}.{key}: numeric budgets are clamped, not rejected"
                        );
                    }
                    for (key, child) in object {
                        check(tool, child, &format!("{path}.{key}"));
                    }
                }
                Value::Array(items) => {
                    for (index, child) in items.iter().enumerate() {
                        check(tool, child, &format!("{path}[{index}]"));
                    }
                }
                _ => {}
            }
        }

        let builtins = builtins();
        for (name, tool) in &builtins.contract.tools {
            check(name, &tool.input_schema, "input");
        }
    }

    #[test]
    fn builtin_contract_names_match_registry() {
        let tools = crate::tool::registry::builtin_code_surface_tools();
        let mut actual: Vec<String> = tools.iter().map(|tool| tool.name().to_string()).collect();
        actual.sort();
        let mut expected: Vec<String> = builtins().contract.tools.keys().cloned().collect();
        expected.sort();
        assert_eq!(actual, expected);
    }

    /// Browser tools are exempt from the Rust-side output drift check. Their
    /// output is the desktop bridge's data, passed through unchanged; Rust owns
    /// no output type for it. Decided in the architect's browser reply (option C).
    /// The runtime output gate still checks every call to these tools.
    const BROWSER_OUTPUT_DRIFT_EXEMPT: &[&str] = &[
        "browser_open",
        "browser_close",
        "browser_list",
        "browser_read",
        "browser_click",
        "browser_type",
    ];

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum OptionalState {
        AllPresent,
        AllAbsent,
    }

    /// Compare a value with its schema node in one optional-field state. An object
    /// key set must equal the properties (all present) or the required names (all
    /// absent), in both directions. An array of objects must hold at least one
    /// item, so item keys are compared. Every declared child is checked the same way.
    fn shape_drift(
        schema: &Value,
        value: &Value,
        state: OptionalState,
        at: &str,
        out: &mut Vec<String>,
    ) {
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            let Some(object) = value.as_object() else {
                out.push(format!("{at}: expected an object"));
                return;
            };
            let declared: BTreeSet<&str> = properties.keys().map(String::as_str).collect();
            let required: BTreeSet<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|names| names.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let expected = match state {
                OptionalState::AllPresent => &declared,
                OptionalState::AllAbsent => &required,
            };
            let actual: BTreeSet<&str> = object.keys().map(String::as_str).collect();
            let missing: Vec<&str> = expected.difference(&actual).copied().collect();
            let unexpected: Vec<&str> = actual.difference(expected).copied().collect();
            if !missing.is_empty() || !unexpected.is_empty() {
                out.push(format!(
                    "{at} ({state:?}): missing {missing:?}, unexpected {unexpected:?}"
                ));
            }
            for (key, child) in object {
                if let Some(child_schema) = properties.get(key) {
                    shape_drift(child_schema, child, state, &format!("{at}.{key}"), out);
                }
            }
        }
        if let Some(items) = schema.get("items")
            && items.get("properties").is_some()
        {
            let Some(array) = value.as_array() else {
                out.push(format!("{at}: expected an array"));
                return;
            };
            if array.is_empty() {
                out.push(format!("{at} ({state:?}): sample has no item to compare"));
            }
            for (index, item) in array.iter().enumerate() {
                shape_drift(items, item, state, &format!("{at}[{index}]"), out);
            }
        }
    }

    /// Every module that owns a built-in code tool output contributes its cases.
    fn builtin_drift_cases() -> Vec<crate::tool::code_contract::DriftCase> {
        let mut cases = Vec::new();
        cases.extend(crate::tools::read::drift_cases());
        cases.extend(crate::tools::glob::drift_cases());
        cases.extend(crate::tools::grep::drift_cases());
        cases.extend(crate::tools::edit::result::drift_cases());
        cases.extend(crate::tools::write::drift_cases());
        cases.extend(crate::tools::command_job::drift_cases());
        cases.extend(crate::tools::webfetch::drift_cases());
        cases.extend(crate::tools::websearch::drift_cases());
        cases.extend(crate::tools::code_search::drift_cases());
        cases.extend(crate::tools::session_search::drift_cases());
        cases.extend(crate::tools::memory::drift_cases());
        cases.extend(crate::tools::lsp::drift_cases());
        cases
    }

    #[test]
    fn builtin_outputs_match_contract_in_both_optional_states() {
        let contract = &builtins().contract.tools;
        let exempt: BTreeSet<&str> = BROWSER_OUTPUT_DRIFT_EXEMPT.iter().copied().collect();
        let browser: BTreeSet<&str> = crate::tools::browser::BROWSER_CODE_TOOL_NAMES
            .iter()
            .copied()
            .collect();
        assert_eq!(
            exempt, browser,
            "browser drift exemption must equal BROWSER_CODE_TOOL_NAMES"
        );
        for name in &exempt {
            assert!(
                contract.contains_key(*name),
                "exempt tool '{name}' is not in the generated contract"
            );
        }

        let cases = builtin_drift_cases();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for case in &cases {
            *counts.entry(case.tool).or_default() += 1;
        }
        let duplicated: Vec<&str> = counts
            .iter()
            .filter(|(_, count)| **count > 1)
            .map(|(name, _)| *name)
            .collect();
        assert!(
            duplicated.is_empty(),
            "duplicate drift cases: {duplicated:?}"
        );
        let covered: BTreeSet<&str> = counts.keys().copied().collect();
        let expected: BTreeSet<&str> = contract
            .keys()
            .map(String::as_str)
            .filter(|name| !exempt.contains(name))
            .collect();
        assert_eq!(
            covered, expected,
            "drift coverage must equal the contract tools minus the browser exemption"
        );

        let surface = build_surface(&crate::tool::registry::builtin_code_surface_tools())
            .expect("assemble builtin surface");
        let mut failures = Vec::new();
        for case in cases {
            let tool = case.tool;
            let states = [
                (OptionalState::AllPresent, case.all_optional_present),
                (OptionalState::AllAbsent, case.all_optional_absent),
            ];
            for (state, result) in states {
                if result.error.is_some()
                    || !matches!(result.level, ToolSignalLevel::Ok | ToolSignalLevel::Warning)
                {
                    failures.push(format!("{tool} {state:?}: sample is not a success result"));
                    continue;
                }
                let Ok(value) = crate::tool::outcome::program_result(&result) else {
                    failures.push(format!("{tool} {state:?}: no program value"));
                    continue;
                };
                if let Some(error) = surface.output_violation(tool, &result) {
                    failures.push(format!("{tool} {state:?}: output gate rejects it: {error}"));
                }
                let mut drift = Vec::new();
                shape_drift(
                    &contract[tool].output_schema,
                    &value,
                    state,
                    "$",
                    &mut drift,
                );
                failures.extend(drift.into_iter().map(|line| format!("{tool} {line}")));
            }
        }
        assert!(
            failures.is_empty(),
            "output contract drift:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn shape_drift_reports_missing_and_unexpected_keys_in_each_state() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "ok": {"const": true},
                "name": {"type": "string"},
                "rows": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"id": {"type": "integer"}, "note": {"type": "string"}},
                        "required": ["id"]
                    }
                }
            },
            "required": ["ok", "rows"]
        });
        let check = |value: Value, state: OptionalState| {
            let mut out = Vec::new();
            shape_drift(&schema, &value, state, "$", &mut out);
            out.join("\n")
        };
        let full = serde_json::json!({"ok": true, "name": "n", "rows": [{"id": 1, "note": "x"}]});
        let bare = serde_json::json!({"ok": true, "rows": [{"id": 1}]});
        assert_eq!(check(full.clone(), OptionalState::AllPresent), "");
        assert_eq!(check(bare.clone(), OptionalState::AllAbsent), "");
        let report = check(bare.clone(), OptionalState::AllPresent);
        assert!(
            report.contains("$ (AllPresent): missing [\"name\"]"),
            "{report}"
        );
        assert!(
            report.contains("$.rows[0] (AllPresent): missing [\"note\"]"),
            "{report}"
        );
        let report = check(full.clone(), OptionalState::AllAbsent);
        assert!(
            report.contains("$ (AllAbsent): missing [], unexpected [\"name\"]"),
            "{report}"
        );
        let extra = serde_json::json!({"ok": true, "name": "n", "rows": [{"id": 1}], "zz": 0});
        assert!(check(extra, OptionalState::AllPresent).contains("unexpected [\"zz\"]"));
        let empty = serde_json::json!({"ok": true, "name": "n", "rows": []});
        assert!(check(empty, OptionalState::AllPresent).contains("sample has no item to compare"));
    }

    #[test]
    fn output_gate_checks_successes_only() {
        let read: Arc<dyn Tool> = Arc::new(crate::tools::read::ReadTool::default());
        let surface = build_surface(&[read]).expect("read surface");
        let wrong = crate::tool::outcome::ok_data(serde_json::json!({"path": 1}));
        assert!(surface.output_violation("read", &wrong).is_some());
        let right = crate::tool::outcome::ok_data(serde_json::json!({
            "path": "a.txt",
            "start_line": 1,
            "lines": [],
            "message": "",
        }));
        assert!(surface.output_violation("read", &right).is_none());
        let failed = ToolCallResult::error_with_kind(
            crate::types::ToolErrorKind::Io,
            "boom".to_string(),
            false,
        );
        assert!(surface.output_violation("read", &failed).is_none());
    }
}
