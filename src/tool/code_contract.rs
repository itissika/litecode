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
    pub tools: BTreeMap<String, CodeToolContract>,
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
    fn builtin_contract_names_match_registry() {
        let tools = crate::tool::registry::builtin_code_surface_tools();
        let mut actual: Vec<String> = tools.iter().map(|tool| tool.name().to_string()).collect();
        actual.sort();
        let mut expected: Vec<String> = builtins().contract.tools.keys().cloned().collect();
        expected.sort();
        assert_eq!(actual, expected);
    }
}
