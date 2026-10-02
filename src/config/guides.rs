//! Product-owned workspace guidance, embedded at compile time.
//!
//! Each topic is a Markdown fragment next to this module, so the prose can be
//! read and edited as prose in dev; `include_str!` ships every fragment inside
//! the binary. Fragments are never user-editable files, and consumers must not
//! read the generated `.litecode/README.md` back from disk.
//!
//! Consumers:
//! - `init_workspace` writes [`WORKSPACE_README`] (compile-time assembly) to
//!   `.litecode/README.md`, regenerated on every open.
//! - the `litecode_workspace` tool prints one topic on demand (`guide <topic>`)
//!   or [`GUIDE_INDEX`] when no topic is given.
//!
//! Fragment contract: every fragment starts with a heading, ends with exactly
//! one newline, and contains no `---` section separator line — the assembler
//! inserts separators so a fragment stays valid when printed on its own.

/// One workspace configuration topic an agent may edit directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideTopic {
    Excludes,
    Mcp,
    CustomTools,
}

impl GuideTopic {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Excludes => "excludes",
            Self::Mcp => "mcp",
            Self::CustomTools => "custom_tools",
        }
    }

    /// Canonical topic name, or `None` when the token names no topic.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "excludes" => Some(Self::Excludes),
            "mcp" => Some(Self::Mcp),
            "custom_tools" => Some(Self::CustomTools),
            _ => None,
        }
    }
}

const INTRO: &str = include_str!("guides/intro.md");
const EDITABLE: &str = include_str!("guides/editable.md");
const EXCLUDES: &str = include_str!("guides/excludes.md");
const MCP: &str = include_str!("guides/mcp.md");
const CUSTOM_TOOLS: &str = include_str!("guides/custom_tools.md");
const READONLY: &str = include_str!("guides/readonly.md");
const QUICKREF: &str = include_str!("guides/quickref.md");

/// The whole `.litecode/README.md`, assembled at compile time.
pub const WORKSPACE_README: &str = concat!(
    include_str!("guides/intro.md"),
    "\n---\n\n",
    include_str!("guides/editable.md"),
    "\n---\n\n",
    include_str!("guides/excludes.md"),
    "\n---\n\n",
    include_str!("guides/mcp.md"),
    "\n---\n\n",
    include_str!("guides/custom_tools.md"),
    "\n---\n\n",
    include_str!("guides/readonly.md"),
    "\n---\n\n",
    include_str!("guides/quickref.md"),
);

/// Topic index printed when `guide` is called without a topic: keeps the
/// panel cheap instead of dumping every fragment into the turn.
pub const GUIDE_INDEX: &str = concat!(
    "# guide\n\n",
    "- `guide excludes` — path / file excludes (`.litecode/excludes.json`)\n",
    "- `guide mcp` — MCP servers (`.litecode/mcp.json`)\n",
    "- `guide custom_tools` — custom tools (`.litecode/custom_tools.json`)\n",
    "\nThese topics are also in `.litecode/README.md` \
     (product-owned, rewritten on every open).\n",
    "\nBoundaries: this tool does not write files or flip switches. \
     Engines stay in Settings → Engines. Enabling an MCP server \
     or a custom tool stays in Settings → Agents.\n",
);

/// One topic's fragment, printed as-is.
pub fn topic(topic: GuideTopic) -> &'static str {
    match topic {
        GuideTopic::Excludes => EXCLUDES,
        GuideTopic::Mcp => MCP,
        GuideTopic::CustomTools => CUSTOM_TOOLS,
    }
}

/// Every fragment, for the contract test and future consumers.
pub const FRAGMENTS: &[(&str, &str)] = &[
    ("intro", INTRO),
    ("editable", EDITABLE),
    ("excludes", EXCLUDES),
    ("mcp", MCP),
    ("custom_tools", CUSTOM_TOOLS),
    ("readonly", READONLY),
    ("quickref", QUICKREF),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{CustomToolDefinition, McpServerDefinition, ToolSchema};
    use crate::workspace::filter::WorkspaceExcludesFile;

    fn object_keys<T: serde::Serialize>(value: &T) -> Vec<String> {
        match serde_json::to_value(value).expect("serialize") {
            serde_json::Value::Object(map) => map.keys().cloned().collect(),
            other => panic!("expected an object, got {other}"),
        }
    }

    #[test]
    fn fragments_follow_the_contract() {
        for (name, text) in FRAGMENTS {
            assert!(text.starts_with('#'), "{name}: must start with a heading");
            assert!(text.ends_with('\n'), "{name}: must end with a newline");
            assert!(
                !text.ends_with("\n\n"),
                "{name}: exactly one trailing newline"
            );
            assert!(
                !text.lines().any(|line| line.trim() == "---"),
                "{name}: fragments must not carry section separators"
            );
        }
    }

    #[test]
    fn readme_assembles_every_fragment_with_separators() {
        for (name, text) in FRAGMENTS {
            assert!(
                WORKSPACE_README.contains(text),
                "README is missing the {name} fragment"
            );
        }
        assert!(WORKSPACE_README.contains("\n---\n\n"));
        assert!(WORKSPACE_README.contains("excludes.json"));
        assert!(WORKSPACE_README.contains("mcp.json"));
        assert!(WORKSPACE_README.contains("custom_tools.json"));
        assert!(WORKSPACE_README.contains("## 只读"));
        assert!(WORKSPACE_README.contains("## Agent 速查"));
    }

    #[test]
    fn topic_fragments_name_every_serde_field() {
        for key in object_keys(&WorkspaceExcludesFile::builtin_defaults()) {
            assert!(
                EXCLUDES.contains(&key),
                "excludes fragment does not document `{key}`"
            );
        }
        for key in object_keys(&McpServerDefinition::default()) {
            assert!(MCP.contains(&key), "mcp fragment does not document `{key}`");
        }
        let custom = CustomToolDefinition {
            name: "demo".into(),
            description: String::new(),
            schema: ToolSchema {
                schema_type: "object".into(),
                properties: serde_json::json!({}),
                required: Vec::new(),
            },
            command: "demo".into(),
            args: Vec::new(),
            timeout: 120,
        };
        for key in object_keys(&custom) {
            assert!(
                CUSTOM_TOOLS.contains(&key),
                "custom tools fragment does not document `{key}`"
            );
        }
    }

    #[test]
    fn topic_fragments_keep_the_id_rule_and_verification_steps() {
        for text in [MCP, CUSTOM_TOOLS] {
            assert!(text.contains("[a-z][a-z0-9_]*"), "id rule missing");
        }
        for text in [EXCLUDES, MCP, CUSTOM_TOOLS] {
            assert!(
                text.contains("litecode_workspace"),
                "verification step missing"
            );
            assert!(text.contains("**怎么验证。**"), "verify section missing");
            assert!(text.contains("**下一步。**"), "next-step section missing");
        }
    }

    #[test]
    fn topic_names_round_trip() {
        for topic in [
            GuideTopic::Excludes,
            GuideTopic::Mcp,
            GuideTopic::CustomTools,
        ] {
            assert_eq!(GuideTopic::parse(topic.as_str()), Some(topic));
        }
        assert_eq!(GuideTopic::parse("nope"), None);
        assert_eq!(
            GuideTopic::parse(" custom_tools "),
            Some(GuideTopic::CustomTools)
        );
    }

    #[test]
    fn guide_index_does_not_mention_the_provider_catalog() {
        assert!(!GUIDE_INDEX.contains("provider"));
        assert!(!WORKSPACE_README.contains("provider-catalog"));
        assert!(!GUIDE_INDEX.contains("writes definitions"));
    }
}
