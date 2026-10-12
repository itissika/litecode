//! Corpus resolution: declaration checks plus one result per cited reference.
//!
//! Pure: every disk / git / parse fact comes through [`Facts`], so the index,
//! the tests and every caller share one set of rules
//! (`docs/knowledge-references.md`). Codes are the wire codes of the
//! knowledge API.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use serde_json::{Value, json};

use super::document::{self, Status};
use super::mentions::{
    Cite, CiteKind, extract_cites, format_line_span, is_knowledge_key, is_workspace_file_ref,
    normalize_key,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

pub type Args = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    /// Node id (unique key, else path). Kept for the `knowledge` tool.
    #[serde(skip)]
    pub node_id: String,
    /// Node path relative to the knowledge root: the node identity on the wire.
    #[serde(skip)]
    pub node_path: String,
    pub severity: Severity,
    pub code: String,
    pub message: String,
    /// Key / path / chain the problem is about (legacy display field).
    #[serde(skip)]
    pub reference: Option<String>,
    /// The citation this issue belongs to. `None` for declaration issues.
    pub ref_key: Option<String>,
    pub args: Args,
}

/// Every `status` a ref (and `code` an issue) can carry. The spec's status
/// table lists exactly these (`spec_tests::status_table_matches_the_codes`).
pub const STATUS_CODES: &[&str] = &[
    "ok",
    "checking",
    "dangling_relation",
    "self_relation",
    "inactive_target",
    "missing_file",
    "symlink_escape",
    "invalid_lines",
    "unreadable_file",
    "unsupported_language",
    "missing_symbol",
    "ambiguous_symbol",
    "symbol_drift",
    "line_out_of_range",
    "invalid_status",
    "empty_key",
    "duplicate_key",
    "filename_mismatch",
];

/// `args.reason` of `invalid_lines` on a range citation: `lines` is not
/// `A` or `A-B` (KR-SYN-5).
pub const INVALID_LINES_FORMAT: &str = "invalid line span";
/// `args.reason` of `invalid_lines` on a symbol citation: symbol citations
/// never take `lines` (KR-SYM-6).
pub const INVALID_LINES_ON_SYMBOL: &str = "lines not allowed on symbol reference";

/// Facts about one cited path, supplied by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFact {
    /// Not known yet (index still building).
    Checking,
    Missing,
    Escape,
    Dir,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFact {
    Checking,
    Unreadable(super::fsref::Unreadable),
    Text { line_count: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolFact {
    Checking,
    Unsupported,
    Missing,
    Ambiguous,
    Present,
    Drifted,
}

pub trait Facts {
    /// Existence of a normalized, legal workspace path.
    fn file(&mut self, path: &str) -> FileFact;
    /// Readability of a path whose `file` fact is `File` or `Dir`.
    fn text(&mut self, path: &str) -> TextFact;
    /// Symbol in a readable text file of a supported language.
    fn symbol(&mut self, path: &str, chain: &str) -> SymbolFact;
}

/// Resolution of one distinct citation inside one node.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RefResult {
    pub ref_key: String,
    pub kind: &'static str,
    pub key: Option<String>,
    pub path: Option<String>,
    pub symbol: Option<String>,
    /// Range citations only: normalized span when valid, the raw attribute
    /// text when not. Always `None` for node, file and symbol citations
    /// (KR-SYM-6; stray text on a symbol is in `args.lines` and `raw`).
    pub lines: Option<String>,
    pub raw: String,
    pub status: &'static str,
    pub severity: Option<Severity>,
    /// Node path of the target node (node refs) or the workspace path cited.
    pub target_path: Option<String>,
    pub args: Args,
    pub message: String,
}

/// Input row: one node of the corpus.
pub struct CheckNode<'a> {
    pub id: &'a str,
    pub key: &'a str,
    pub value: &'a str,
    pub status: Status,
    pub invalid_status: Option<&'a str>,
    pub path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Resolution {
    /// Per node, in input order: declaration issues then citation issues.
    pub issues: Vec<Vec<Issue>>,
    /// Per node, distinct citations in first-seen order.
    pub refs: Vec<Vec<RefResult>>,
}

impl Resolution {
    pub fn flat_issues(&self) -> Vec<Issue> {
        self.issues.iter().flatten().cloned().collect()
    }
}

/// Resolve every node. `cites` supplies the extracted citations per node
/// (callers cache extraction); `None` extracts from `value`.
pub fn resolve(
    nodes: &[CheckNode<'_>],
    cites: Option<&[std::sync::Arc<Vec<Cite>>]>,
    facts: &mut dyn Facts,
) -> Resolution {
    let mut key_counts: HashMap<String, usize> = HashMap::new();
    for node in nodes {
        let key = normalize_key(node.key);
        if is_knowledge_key(&key) {
            *key_counts.entry(key).or_insert(0) += 1;
        }
    }
    // KR-NODE-4: the first node by path wins; input is in path order.
    let mut by_key: HashMap<String, (Status, &str)> = HashMap::new();
    for node in nodes {
        let key = normalize_key(node.key);
        if is_knowledge_key(&key) {
            by_key.entry(key).or_insert((node.status, node.path));
        }
    }

    let mut out = Resolution::default();
    for (index, node) in nodes.iter().enumerate() {
        let key = normalize_key(node.key);
        let mut issues = declaration_issues(node, &key, &key_counts);
        let owned;
        let node_cites: &[Cite] = match cites {
            Some(all) => all[index].as_slice(),
            None => {
                owned = extract_cites(node.value);
                &owned
            }
        };
        let mut refs: Vec<RefResult> = Vec::new();
        let mut slot: HashMap<String, usize> = HashMap::new();
        for cite in node_cites {
            let ref_key = cite.ref_key();
            let result = resolve_cite(cite, ref_key.clone(), node, &key, &by_key, facts);
            match slot.get(&ref_key) {
                // KR-VAL-2: one result per ref_key; a later error replaces an
                // earlier non-error (e.g. the same chain once with bad lines).
                Some(&at) => {
                    if rank(&result) > rank(&refs[at]) {
                        refs[at] = result;
                    }
                }
                None => {
                    slot.insert(ref_key, refs.len());
                    refs.push(result);
                }
            }
        }
        for result in &refs {
            if let Some(severity) = result.severity {
                issues.push(Issue {
                    node_id: node.id.to_string(),
                    node_path: node.path.to_string(),
                    severity,
                    code: result.status.to_string(),
                    message: result.message.clone(),
                    reference: issue_reference(result),
                    ref_key: Some(result.ref_key.clone()),
                    args: result.args.clone(),
                });
            }
        }
        out.issues.push(issues);
        out.refs.push(refs);
    }
    out
}

/// Flat issue list, the legacy shape (tests, tool).
pub fn validate_with(nodes: &[CheckNode<'_>], facts: &mut dyn Facts) -> Vec<Issue> {
    resolve(nodes, None, facts).flat_issues()
}

fn rank(result: &RefResult) -> u8 {
    match result.severity {
        Some(Severity::Error) => 2,
        Some(Severity::Warning) => 1,
        None => 0,
    }
}

fn issue_reference(result: &RefResult) -> Option<String> {
    match result.kind {
        "node" => result.key.clone(),
        "symbol" if result.status != "missing_file" && result.status != "symlink_escape" => {
            result.symbol.clone()
        }
        _ => result.path.clone(),
    }
}

fn declaration_issues(
    node: &CheckNode<'_>,
    key: &str,
    key_counts: &HashMap<String, usize>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut push =
        |severity, code: &str, message: String, reference: Option<String>, args: Args| {
            issues.push(Issue {
                node_id: node.id.to_string(),
                node_path: node.path.to_string(),
                severity,
                code: code.into(),
                message,
                reference,
                ref_key: None,
                args,
            })
        };
    if let Some(raw) = node.invalid_status {
        push(
            Severity::Error,
            "invalid_status",
            format!("Status \"{raw}\" is not valid. Use enabled, disabled, or pending."),
            Some(raw.to_string()),
            args([("status", json!(raw))]),
        );
    }
    if key.is_empty() {
        push(
            Severity::Error,
            "empty_key",
            "Declaration is missing.".into(),
            None,
            Args::new(),
        );
    } else if !is_knowledge_key(key) {
        push(
            Severity::Error,
            "empty_key",
            format!("Key \"{key}\" is not valid."),
            Some(key.to_string()),
            args([("key", json!(key))]),
        );
    } else if key_counts.get(key).copied().unwrap_or(0) > 1 {
        push(
            Severity::Error,
            "duplicate_key",
            format!("Key \"{key}\" is duplicated."),
            Some(key.to_string()),
            args([("key", json!(key))]),
        );
    } else if !node.path.is_empty() && document::file_stem(node.path) != key {
        let stem = document::file_stem(node.path);
        push(
            Severity::Warning,
            "filename_mismatch",
            format!("Filename \"{stem}\" does not match declaration \"{key}\"."),
            Some(key.to_string()),
            args([("key", json!(key)), ("filename", json!(stem))]),
        );
    }
    issues
}

fn args<const N: usize>(pairs: [(&str, Value); N]) -> Args {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

struct Verdict {
    status: &'static str,
    severity: Option<Severity>,
    message: String,
    extra: Args,
}

fn ok() -> Verdict {
    Verdict {
        status: "ok",
        severity: None,
        message: String::new(),
        extra: Args::new(),
    }
}

fn checking() -> Verdict {
    Verdict {
        status: "checking",
        severity: None,
        message: String::new(),
        extra: Args::new(),
    }
}

fn error(status: &'static str, message: String) -> Verdict {
    Verdict {
        status,
        severity: Some(Severity::Error),
        message,
        extra: Args::new(),
    }
}

fn resolve_cite(
    cite: &Cite,
    ref_key: String,
    node: &CheckNode<'_>,
    own_key: &str,
    by_key: &HashMap<String, (Status, &str)>,
    facts: &mut dyn Facts,
) -> RefResult {
    let path = cite.norm_path();
    // `lines` text as written: the span of a range, or the raw text that is
    // not allowed (an invalid span, or any `lines` on a symbol, KR-SYM-6).
    let lines_text = match (cite.lines, cite.bad_lines.as_deref()) {
        (Some(span), _) => Some(format_line_span(span)),
        (None, Some(raw)) => Some(raw.to_string()),
        (None, None) => None,
    };
    // The `lines` field belongs to range citations only; a symbol citation
    // never has one (the stray text stays in `args.lines` and `raw`).
    let lines_field = if cite.kind == CiteKind::Range {
        lines_text.clone()
    } else {
        None
    };
    let mut base = Args::new();
    let mut target_path = None;
    let verdict = match cite.kind {
        CiteKind::Node => {
            let key = cite.key.clone().unwrap_or_default();
            base.insert("key".into(), json!(key));
            if key == own_key {
                error("self_relation", "Citation points at itself.".into())
            } else {
                match by_key.get(&key) {
                    None => error(
                        "dangling_relation",
                        format!("Citation \"{key}\" does not exist."),
                    ),
                    Some((target, at)) => {
                        target_path = Some(at.to_string());
                        base.insert("target_status".into(), json!(target.as_str()));
                        if node.status == Status::Enabled && *target != Status::Enabled {
                            Verdict {
                                status: "inactive_target",
                                severity: Some(Severity::Warning),
                                message: format!("Cites {} \"{key}\".", target.as_str()),
                                extra: Args::new(),
                            }
                        } else {
                            ok()
                        }
                    }
                }
            }
        }
        CiteKind::File | CiteKind::Symbol | CiteKind::Range => {
            let path = path.clone().unwrap_or_default();
            base.insert("path".into(), json!(path));
            if let Some(chain) = &cite.symbol {
                base.insert("symbol".into(), json!(chain));
            }
            if let Some(lines) = &lines_text {
                base.insert("lines".into(), json!(lines));
            }
            target_path = Some(path.clone());
            resolve_path_cite(cite, &path, facts)
        }
    };
    let mut args = base;
    args.extend(verdict.extra);
    RefResult {
        ref_key,
        kind: cite.kind.as_str(),
        key: cite.key.clone(),
        path,
        symbol: cite.symbol.clone(),
        lines: lines_field,
        raw: cite.raw.clone(),
        status: verdict.status,
        severity: verdict.severity,
        target_path,
        args,
        message: verdict.message,
    }
}

/// KR order: path → lines format → readability → language → symbol → bounds → drift.
fn resolve_path_cite(cite: &Cite, path: &str, facts: &mut dyn Facts) -> Verdict {
    let file = if is_workspace_file_ref(path) {
        facts.file(path)
    } else {
        FileFact::Missing
    };
    match file {
        FileFact::Checking => return checking(),
        FileFact::Missing => {
            return error("missing_file", format!("File \"{path}\" does not exist."));
        }
        FileFact::Escape => {
            return error(
                "symlink_escape",
                format!("\"{path}\" is a link that resolves outside the workspace."),
            );
        }
        FileFact::Dir | FileFact::File => {}
    }
    if let Some(raw) = &cite.bad_lines {
        let mut verdict = if cite.kind == CiteKind::Symbol {
            // KR-SYM-6: symbol citations locate by chain only.
            error(
                "invalid_lines",
                format!(
                    "Symbol citations do not take lines (lines=\"{raw}\"). Remove lines, or cite a line range as [@ file=\"{path}\" lines=\"A-B\"]."
                ),
            )
        } else {
            error(
                "invalid_lines",
                format!(
                    "Lines \"{raw}\" are not valid. Write lines=\"A\" or lines=\"A-B\" (decimal, 1 <= A <= B)."
                ),
            )
        };
        let reason = if cite.kind == CiteKind::Symbol {
            INVALID_LINES_ON_SYMBOL
        } else {
            INVALID_LINES_FORMAT
        };
        verdict.extra.insert("reason".into(), json!(reason));
        return verdict;
    }
    if cite.kind == CiteKind::File {
        return ok();
    }
    let line_count = match facts.text(path) {
        TextFact::Checking => return checking(),
        TextFact::Unreadable(why) => {
            let mut verdict = error(
                "unreadable_file",
                format!("\"{path}\" is not a readable text file ({}).", why.reason()),
            );
            verdict.extra.insert("reason".into(), json!(why.reason()));
            return verdict;
        }
        TextFact::Text { line_count } => line_count,
    };
    if let Some(chain) = &cite.symbol {
        return match facts.symbol(path, chain) {
            SymbolFact::Checking => checking(),
            SymbolFact::Unsupported => error(
                "unsupported_language",
                format!("该语言不支持符号解析 (\"{path}\")."),
            ),
            SymbolFact::Missing => error(
                "missing_symbol",
                format!("Symbol \"{chain}\" in \"{path}\" does not exist."),
            ),
            SymbolFact::Ambiguous => error(
                "ambiguous_symbol",
                format!("Symbol \"{chain}\" in \"{path}\" is not unique."),
            ),
            SymbolFact::Drifted => Verdict {
                status: "symbol_drift",
                severity: Some(Severity::Warning),
                message: format!("Symbol \"{chain}\" in \"{path}\" differs from HEAD."),
                extra: Args::new(),
            },
            SymbolFact::Present => ok(),
        };
    }
    // Range citation (KR-RANGE-2).
    let Some(span) = cite.lines else {
        return ok();
    };
    if span.end > line_count {
        let mut verdict = error(
            "line_out_of_range",
            format!(
                "Lines {} are outside \"{path}\", which has {line_count} lines.",
                format_line_span(span)
            ),
        );
        verdict.extra.insert("line_count".into(), json!(line_count));
        return verdict;
    }
    ok()
}
