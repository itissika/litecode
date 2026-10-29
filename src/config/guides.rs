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
    Apps,
}

impl GuideTopic {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Excludes => "excludes",
            Self::Mcp => "mcp",
            Self::CustomTools => "custom_tools",
            Self::Apps => "apps",
        }
    }

    /// Canonical topic name, or `None` when the token names no topic.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "excludes" => Some(Self::Excludes),
            "mcp" => Some(Self::Mcp),
            "custom_tools" => Some(Self::CustomTools),
            "apps" => Some(Self::Apps),
            _ => None,
        }
    }
}

const INTRO: &str = include_str!("guides/intro.md");
const EDITABLE: &str = include_str!("guides/editable.md");
const EXCLUDES: &str = include_str!("guides/excludes.md");
const MCP: &str = include_str!("guides/mcp.md");
const CUSTOM_TOOLS: &str = include_str!("guides/custom_tools.md");
const APPS: &str = include_str!("guides/apps.md");
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
    include_str!("guides/apps.md"),
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
    "- `guide apps` — local apps (`.litecode/apps.json`)\n",
    "\nLive app state: `apps [id]` lists built-in and custom apps, status, URL, and last error.\n",
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
        GuideTopic::Apps => APPS,
    }
}

/// Every fragment, for the contract test and future consumers.
pub const FRAGMENTS: &[(&str, &str)] = &[
    ("intro", INTRO),
    ("editable", EDITABLE),
    ("excludes", EXCLUDES),
    ("mcp", MCP),
    ("custom_tools", CUSTOM_TOOLS),
    ("apps", APPS),
    ("readonly", READONLY),
    ("quickref", QUICKREF),
];

#[cfg(test)]
mod tests {
    
    
    

}
