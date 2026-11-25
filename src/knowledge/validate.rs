//! Corpus resolution: declaration checks plus one result per cited reference.
//!
//! Pure: every disk / parse fact comes through [`Facts`], so the index,
//! the tests and every caller share one set of rules. Codes are the wire codes
//! of the knowledge API.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use serde_json::{Value, json};

use super::corpus::is_valid_node_identity;
use super::document::{self, Status};
use super::mentions::{
    Cite, CiteKind, extract_cites, format_line_span, is_knowledge_key, is_workspace_file_ref,
    normalize_key,
};
use super::root;

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
    /// Key / path / anchor the problem is about (legacy display field).
    #[serde(skip)]
    pub reference: Option<String>,
    /// The citation this issue belongs to. `None` for declaration issues.
    pub ref_key: Option<String>,
    pub args: Args,
}

/// Every `status` a ref (and `code` an issue) can carry.
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
    "line_out_of_range",
    "invalid_status",
    "empty_key",
    "duplicate_key",
    "filename_mismatch",
];

/// `args.reason` of `invalid_lines` on a range citation: fragment is not
/// `#L12`, `#L12-30`, or `#L12-L30`.
pub const INVALID_LINES_FORMAT: &str = "invalid line span";

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
    /// Range citations only: normalized span when valid, raw fragment when not.
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

struct TargetTables<'a> {
    corpus_root: Option<&'a str>,
    /// Valid citeable nodes: rel path -> (normalized key, status, rel path).
    valid_by_path: HashMap<&'a str, (String, Status, &'a str)>,
    /// Bad nodes (filename_mismatch or invalid_key): rel path -> (rel path, reason).
    bad_by_path: HashMap<&'a str, (&'a str, &'static str)>,
}

/// Resolve every node. `cites` supplies the extracted citations per node
/// (callers cache extraction); `None` extracts from `value`.
pub fn resolve(
    nodes: &[CheckNode<'_>],
    cites: Option<&[std::sync::Arc<Vec<Cite>>]>,
    corpus_root: Option<&str>,
    facts: &mut dyn Facts,
) -> Resolution {
    let mut key_counts: HashMap<String, usize> = HashMap::new();
    for node in nodes {
        let key = normalize_key(node.key);
        if is_knowledge_key(&key) {
            *key_counts.entry(key).or_insert(0) += 1;
        }
    }
    let mut valid_by_path: HashMap<&str, (String, Status, &str)> = HashMap::new();
    let mut bad_by_path: HashMap<&str, (&str, &'static str)> = HashMap::new();
    for node in nodes {
        let key = normalize_key(node.key);
        if is_valid_node_identity(node.path, &key) {
            valid_by_path.insert(node.path, (key, node.status, node.path));
        } else {
            let reason = if key.is_empty() || !is_knowledge_key(&key) {
                "invalid_key"
            } else {
                "filename_mismatch"
            };
            bad_by_path.insert(node.path, (node.path, reason));
        }
    }
    let targets = TargetTables {
        corpus_root,
        valid_by_path,
        bad_by_path,
    };

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
            let result = resolve_cite(cite, ref_key.clone(), node, &targets, facts);
            match slot.get(&ref_key) {
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

/// Flat issue list helper (tests).
pub fn validate_with(
    nodes: &[CheckNode<'_>],
    corpus_root: Option<&str>,
    facts: &mut dyn Facts,
) -> Vec<Issue> {
    resolve(nodes, None, corpus_root, facts).flat_issues()
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
        "node" => result.key.clone().or_else(|| result.path.clone()),
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
            Severity::Error,
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
    targets: &TargetTables<'_>,
    facts: &mut dyn Facts,
) -> RefResult {
    let norm_path = cite.norm_path();
    let lines_text = match (cite.lines, cite.bad_lines.as_deref()) {
        (Some(span), _) => Some(format_line_span(span)),
        (None, Some(raw)) => Some(raw.to_string()),
        (None, None) => None,
    };
    let lines_field = if cite.kind == CiteKind::Range {
        lines_text.clone()
    } else {
        None
    };

    // Fold bare-path `.md` citations under the active knowledge root into node citations.
    if cite.kind == CiteKind::File
        && is_workspace_file_ref(&norm_path)
        && let Some(rel) = root::strip_corpus_prefix(&norm_path, targets.corpus_root)
        && rel.to_ascii_lowercase().ends_with(".md")
    {
        if let Some((target_key, target_status, target_rel)) = targets.valid_by_path.get(rel) {
            let mut base = Args::new();
            base.insert("key".into(), json!(target_key));
            base.insert("path".into(), json!(norm_path));
            let verdict = if *target_rel == node.path {
                error("self_relation", "Citation points at itself.".into())
            } else {
                base.insert("target_status".into(), json!(target_status.as_str()));
                if node.status == Status::Enabled && *target_status != Status::Enabled {
                    Verdict {
                        status: "inactive_target",
                        severity: Some(Severity::Warning),
                        message: format!("Cites {} \"{target_key}\".", target_status.as_str()),
                        extra: Args::new(),
                    }
                } else {
                    ok()
                }
            };
            base.extend(verdict.extra);
            return RefResult {
                ref_key,
                kind: CiteKind::Node.as_str(),
                key: Some(target_key.clone()),
                path: Some(norm_path),
                symbol: None,
                lines: None,
                raw: cite.raw.clone(),
                status: verdict.status,
                severity: verdict.severity,
                target_path: Some((*target_rel).to_string()),
                args: base,
                message: verdict.message,
            };
        }
        if let Some((bad_rel, reason)) = targets.bad_by_path.get(rel) {
            let mut base = Args::new();
            base.insert("path".into(), json!(norm_path));
            base.insert("reason".into(), json!(reason));
            let message = if *reason == "filename_mismatch" {
                format!(
                    "Target \"{norm_path}\" is not a valid knowledge node (filename does not match declaration)."
                )
            } else {
                format!("Target \"{norm_path}\" is not a valid knowledge node.")
            };
            return RefResult {
                ref_key,
                kind: CiteKind::Node.as_str(),
                key: None,
                path: Some(norm_path),
                symbol: None,
                lines: None,
                raw: cite.raw.clone(),
                status: "dangling_relation",
                severity: Some(Severity::Error),
                target_path: Some((*bad_rel).to_string()),
                args: base,
                message,
            };
        }
    }

    // Ordinary file / directory / symbol / line-range resolution.
    let mut base = Args::new();
    base.insert("path".into(), json!(norm_path));
    if let Some(symbol) = &cite.symbol {
        base.insert("symbol".into(), json!(symbol));
    }
    if let Some(lines) = &lines_text {
        base.insert("lines".into(), json!(lines));
    }
    let verdict = resolve_path_cite(cite, &norm_path, facts);
    base.extend(verdict.extra);
    RefResult {
        ref_key,
        kind: cite.kind.as_str(),
        key: None,
        path: Some(norm_path.clone()),
        symbol: cite.symbol.clone(),
        lines: lines_field,
        raw: cite.raw.clone(),
        status: verdict.status,
        severity: verdict.severity,
        target_path: Some(norm_path),
        args: base,
        message: verdict.message,
    }
}

/// KR order: path → lines format → readability → language → symbol → bounds.
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
        let mut verdict = error(
            "invalid_lines",
            format!("Lines \"{raw}\" are not valid. Write #L12 or #L12-L30 (decimal, 1 <= A <= B)."),
        );
        verdict.extra.insert("reason".into(), json!(INVALID_LINES_FORMAT));
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
    if let Some(symbol) = &cite.symbol {
        return match facts.symbol(path, symbol) {
            SymbolFact::Checking => checking(),
            SymbolFact::Unsupported => error(
                "unsupported_language",
                format!("该语言不支持符号解析 (\"{path}\")."),
            ),
            SymbolFact::Missing => error(
                "missing_symbol",
                format!("Symbol \"{symbol}\" in \"{path}\" does not exist."),
            ),
            SymbolFact::Ambiguous => error(
                "ambiguous_symbol",
                format!("Symbol \"{symbol}\" in \"{path}\" is not unique."),
            ),
            SymbolFact::Present => ok(),
        };
    }
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
