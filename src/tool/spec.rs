//! Call-surface contract shared by runtime projection and settings.
//!
//! A function is either model-facing (direct) or reachable inside `run_code`
//! (code). Direct functions have only an input contract. Code functions also
//! own their result schema. Admission and SDK rendering consume this type; the
//! live `Tool` remains the execution adapter.

use serde_json::Value;

use crate::config::schema::{CallSurface, CustomToolDefinition};
use crate::context_pipeline::Context;
use crate::tool::trait_::Tool;

/// Context used only to ask a tool for its description. Code-surface text must
/// not pick up the caller's cwd, permission, or depth.
fn description_context() -> Context {
    Context {
        cwd: std::path::PathBuf::new(),
        workspace_paths: crate::config::WorkspacePaths::for_legacy_root(std::path::Path::new(".")),
        agents_md: None,
        claude_md: None,
    }
}

/// The input and description of a direct (model-facing) function.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectFunctionSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// The input, output, and description of a function bundled into `run_code`.
#[derive(Debug, Clone, PartialEq)]
pub struct CodeFunctionSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
}

/// One function contract. The variant is the surface; there is no optional
/// `surface` field or optional result schema on a direct function.
#[derive(Debug, Clone, PartialEq)]
pub enum FunctionSpec {
    Direct(DirectFunctionSpec),
    Code(CodeFunctionSpec),
}

impl FunctionSpec {
    /// Project a live tool's own declaration into a function contract.
    ///
    /// A code tool without a result schema is an admission failure, not a
    /// direct tool.
    pub fn from_tool(tool: &dyn Tool) -> Result<Self, String> {
        let name = tool.name().to_string();
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(format!("tool '{name}' has an unsafe name"));
        }
        let description = tool.description(&description_context());
        let input_schema = tool.schema();
        match tool.surface() {
            CallSurface::Direct => Ok(Self::Direct(DirectFunctionSpec {
                name,
                description,
                input_schema,
            })),
            CallSurface::Code => {
                let output_schema = tool
                    .data_schema()
                    .ok_or_else(|| format!("tool '{name}' has no data schema"))?;
                Ok(Self::Code(CodeFunctionSpec {
                    name,
                    description,
                    input_schema,
                    output_schema,
                }))
            }
        }
    }

    /// Build a contract straight from a stored custom definition. This is the
    /// settings/admission path: no fake Tool is created just to carry schemas.
    pub fn from_custom_definition(def: &CustomToolDefinition) -> Result<Self, String> {
        let name = custom_function_name(def);
        let description = custom_description(def);
        let input_schema = custom_input_schema(def);
        match def.effective_surface() {
            CallSurface::Direct => Ok(Self::Direct(DirectFunctionSpec {
                name,
                description,
                input_schema,
            })),
            CallSurface::Code => {
                let output_schema = def.output_schema.clone().ok_or_else(|| {
                    format!("code custom tool '{name}' must declare output_schema")
                })?;
                Ok(Self::Code(CodeFunctionSpec {
                    name,
                    description,
                    input_schema,
                    output_schema,
                }))
            }
        }
    }

    pub fn surface(&self) -> CallSurface {
        match self {
            Self::Direct(_) => CallSurface::Direct,
            Self::Code(_) => CallSurface::Code,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Direct(spec) => &spec.name,
            Self::Code(spec) => &spec.name,
        }
    }

    pub fn input_schema(&self) -> &Value {
        match self {
            Self::Direct(spec) => &spec.input_schema,
            Self::Code(spec) => &spec.input_schema,
        }
    }

    pub fn output_schema(&self) -> Option<&Value> {
        match self {
            Self::Direct(_) => None,
            Self::Code(spec) => Some(&spec.output_schema),
        }
    }

    pub fn as_code(&self) -> Option<&CodeFunctionSpec> {
        match self {
            Self::Direct(_) => None,
            Self::Code(spec) => Some(spec),
        }
    }

    pub fn into_code(self) -> Option<CodeFunctionSpec> {
        match self {
            Self::Direct(_) => None,
            Self::Code(spec) => Some(spec),
        }
    }
}

/// Code names are canonical: a code suite member is `suite.member` even when
/// the suite has one member. Direct suite members stay flat.
pub fn custom_function_name(def: &CustomToolDefinition) -> String {
    match (def.effective_surface(), def.suite.as_deref().map(str::trim)) {
        (CallSurface::Code, Some(suite)) if !suite.is_empty() => {
            format!("{suite}.{}", def.name)
        }
        _ => def.name.clone(),
    }
}

/// Input shape of a custom function. Direct tools keep the background knob;
/// code calls are synchronous and cannot start a job.
pub(crate) fn custom_input_schema(config: &CustomToolDefinition) -> Value {
    let mut schema = config.to_json_schema();
    if config.effective_surface() == CallSurface::Direct
        && let Some(obj) = schema.as_object_mut()
    {
        let props = obj
            .entry("properties")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut();
        if let Some(props) = props {
            props.insert(
                "run_in_background".into(),
                serde_json::json!({
                    "type": "boolean",
                    "description": "When true, start as a background job and return job_id immediately; result is delivered later via CustomToolSettled mailbox (default false)."
                }),
            );
        }
    }
    schema
}

pub(crate) fn custom_description(config: &CustomToolDefinition) -> String {
    let trimmed = config.description.trim();
    if trimmed.is_empty() {
        format!("External tool: {}", config.name)
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::ToolSchema;

    struct DirectWithData;

    impl Tool for DirectWithData {
        fn name(&self) -> &str {
            "direct_data"
        }

        fn schema(&self) -> Value {
            serde_json::json!({"type": "object", "properties": {}})
        }

        fn data_schema(&self) -> Option<Value> {
            Some(serde_json::json!({"type": "object", "properties": {"x": {"type": "string"}}}))
        }

        fn description(&self, _ctx: &Context) -> String {
            "A direct tool with an optional data schema.".into()
        }
    }

    fn custom(surface: CallSurface, suite: Option<&str>, output: bool) -> CustomToolDefinition {
        CustomToolDefinition {
            name: "probe".into(),
            description: String::new(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "echo".into(),
            args: Vec::new(),
            timeout: 120,
            rules: Vec::new(),
            surface: Some(surface),
            suite: suite.map(str::to_string),
            output_schema: output.then(|| {
                serde_json::json!({"type": "object", "properties": {}, "required": []})
            }),
        }
    }

    #[test]
    fn a_direct_tool_never_enters_the_code_contract() {
        let spec = FunctionSpec::from_tool(&DirectWithData).unwrap();
        assert_eq!(spec.surface(), CallSurface::Direct);
        assert!(spec.output_schema().is_none());
    }

    #[test]
    fn code_suite_members_use_the_canonical_namespace() {
        let spec = FunctionSpec::from_custom_definition(&custom(
            CallSurface::Code,
            Some("deck"),
            true,
        ))
        .unwrap();
        assert_eq!(spec.name(), "deck.probe");
    }

    #[test]
    fn direct_custom_keeps_background_and_code_custom_does_not() {
        let direct = FunctionSpec::from_custom_definition(&custom(
            CallSurface::Direct,
            None,
            false,
        ))
        .unwrap();
        assert_eq!(direct.name(), "probe");
        assert!(
            direct.input_schema()["properties"]
                .get("run_in_background")
                .is_some()
        );
        assert!(direct.output_schema().is_none());

        let code = FunctionSpec::from_custom_definition(&custom(CallSurface::Code, None, true))
            .unwrap();
        assert!(
            code.input_schema()["properties"]
                .get("run_in_background")
                .is_none()
        );
        assert!(code.output_schema().is_some());
    }

    #[test]
    fn code_custom_without_output_is_rejected() {
        let error =
            FunctionSpec::from_custom_definition(&custom(CallSurface::Code, None, false))
                .unwrap_err();
        assert!(error.contains("output_schema"), "{error}");
    }
}

