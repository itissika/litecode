//! Resident knowledge index: the backend single source of truth.
//!
//! One [`Engine`] per workspace keeps every expensive fact cached:
//! knowledge markdown by (len, mtime), cited files by exact-name stamp
//! (content, line index, tree-sitter scopes parsed at most once per stamp),
//! and symbol verdicts keyed by file generation. A refresh re-stats only what a
//! hint names (or everything on a full sweep), reparses only what changed, then
//! re-runs the pure resolver over cached facts.
//!
//! [`KnowledgeIndex`] owns an engine plus a background worker fed by the
//! workspace change bus, a periodic full sweep, and publishes immutable
//! versioned [`Snapshot`]s. The per-turn knowledge note, the HTTP API and the
//! `knowledge` tool all read those snapshots; nothing on the turn path parses,
//! spawns git, or touches the disk (see `docs/knowledge-references.md` §8-9).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, LazyLock, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

use super::corpus::Corpus;
use super::document::{self, ParsedFile};
use super::fsref::{self, Kind, Listings, Stamp, TextRead, Unreadable};
use super::mentions::{Cite, extract_cites, is_workspace_file_ref, normalize_ref_path};
use super::root::{self, KnowledgeRoot};
use super::status::{self, Note};
use super::validate::{
    self, CheckNode, Facts, FileFact, Issue, RefResult, Severity, SymbolFact, TextFact,
};
use crate::engines::code_search::enclosing::{
    ScopeEntry, ScopeMatch, list_scopes, match_scopes, supports_scopes,
};
use crate::telemetry::latency;

/// Full background sweep period: the backstop for paths the watcher does not
/// report (watcher excludes, repos above the workspace, lagged bus).
pub const SWEEP_EVERY: Duration = Duration::from_secs(2);
/// Longest a turn waits for a queued recompute before using the last snapshot.
pub const TURN_WAIT: Duration = Duration::from_millis(50);

// ------------------------------------------------------------------ engine

#[derive(Debug, Clone, Default)]
pub struct Hint {
    pub full: bool,
    pub paths: HashSet<String>,
}

impl Hint {
    pub fn full() -> Self {
        Self {
            full: true,
            paths: HashSet::new(),
        }
    }

    fn merge(&mut self, other: Hint) {
        self.full |= other.full;
        self.paths.extend(other.paths);
    }

    fn is_empty(&self) -> bool {
        !self.full && self.paths.is_empty()
    }
}

/// Cumulative work counters (tests and latency lines read them).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub md_parses: u64,
    pub file_reads: u64,
    pub stats: u64,
    pub ts_parses: u64,
    pub refreshes: u64,
}

struct TextEntry {
    content: String,
    line_count: u32,
    /// `None` = unsupported language. Parsed at most once per entry.
    scopes: OnceLock<Option<Arc<Vec<ScopeEntry>>>>,
}

impl TextEntry {
    fn new(content: String) -> Self {
        let line_count = fsref::line_count(&content);
        Self {
            content,
            line_count,
            scopes: OnceLock::new(),
        }
    }

    fn scopes(&self, path: &str, parses: &mut u64) -> Option<Arc<Vec<ScopeEntry>>> {
        self.scopes
            .get_or_init(|| {
                if !supports_scopes(path) {
                    return None;
                }
                *parses += 1;
                Some(Arc::new(list_scopes(path, &self.content)))
            })
            .clone()
    }
}

enum Loaded {
    Unreadable(Unreadable),
    Text(Arc<TextEntry>),
}

struct FileEntry {
    stamp: Stamp,
    /// Bumped whenever the stamp changes; keys the symbol verdict cache.
    generation: u64,
    loaded: Option<Loaded>,
}

#[derive(Clone, Copy)]
struct SymbolDetail {
    fact: SymbolFact,
    start_line: Option<u32>,
    end_line: Option<u32>,
}

struct MdEntry {
    len: u64,
    mtime: Option<SystemTime>,
    parsed: ParsedFile,
    cites: Arc<Vec<Cite>>,
}

pub struct Engine {
    workspace: PathBuf,
    kroot: Option<String>,
    /// Keyed by path relative to the knowledge root.
    md: HashMap<String, MdEntry>,
    dirs: Vec<String>,
    corpus: Corpus,
    cites: Vec<Arc<Vec<Cite>>>,
    files: HashMap<String, FileEntry>,
    next_generation: u64,
    symbols: HashMap<(String, String), (u64, SymbolDetail)>,
    pub counters: Counters,
    corpus_loaded: bool,
}

impl Engine {
    pub fn new(workspace: &Path) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            kroot: None,
            md: HashMap::new(),
            dirs: Vec::new(),
            corpus: Corpus::empty(),
            cites: Vec::new(),
            files: HashMap::new(),
            next_generation: 1,
            symbols: HashMap::new(),
            counters: Counters::default(),
            corpus_loaded: false,
        }
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Corpus only, citations to paths left `checking` (cold start, KR-INV-3).
    pub fn preview(&mut self) -> Content {
        self.reload_corpus();
        let resolution = {
            let checks = checks_of(&self.corpus);
            validate::resolve(&checks, Some(self.cites.as_slice()), &mut Pending)
        };
        self.content(resolution, false)
    }

    /// Bring cached facts up to date for `hint` and resolve everything.
    pub fn refresh(&mut self, hint: &Hint) -> Content {
        let mut stages = latency::Stages::new("knowledge_index.refresh", None);
        let before = self.counters;
        self.counters.refreshes += 1;
        let corpus_touched = !self.corpus_loaded
            || hint.full
            || hint.paths.iter().any(|path| touches_knowledge_root(path));
        if corpus_touched {
            self.reload_corpus();
        }
        stages.mark("corpus");
        self.sweep_files(hint);
        stages.mark("stat");
        let corpus = std::mem::replace(&mut self.corpus, Corpus::empty());
        let cites = std::mem::take(&mut self.cites);
        let resolution = {
            let checks = checks_of(&corpus);
            let mut facts = EngineFacts { engine: self };
            validate::resolve(&checks, Some(cites.as_slice()), &mut facts)
        };
        self.corpus = corpus;
        self.cites = cites;
        stages.mark("resolve");
        self.prune();
        let content = self.content(resolution, true);
        stages.mark("assemble");
        if latency::enabled() {
            let c = self.counters;
            stages.note("full", hint.full);
            stages.note("hinted_paths", hint.paths.len());
            stages.note("nodes", self.corpus.nodes.len());
            stages.note("md_parses", c.md_parses - before.md_parses);
            stages.note("stats", c.stats - before.stats);
            stages.note("file_reads", c.file_reads - before.file_reads);
            stages.note("ts_parses", c.ts_parses - before.ts_parses);
        }
        content
    }

    // ---------------------------------------------------------- corpus

    fn reload_corpus(&mut self) {
        self.corpus_loaded = true;
        let kroot = match root::locate(&self.workspace) {
            KnowledgeRoot::Present(root) => Some(root),
            KnowledgeRoot::Missing => None,
        };
        if kroot != self.kroot {
            self.md.clear();
            self.kroot = kroot.clone();
        }
        let Some(kroot) = kroot else {
            self.dirs.clear();
            self.corpus = Corpus::empty();
            self.cites.clear();
            return;
        };
        let mut seen = HashSet::new();
        let mut dirs = Vec::new();
        let base = self.workspace.join(&kroot);
        self.walk(&base, "", &mut seen, &mut dirs);
        self.md.retain(|path, _| seen.contains(path));
        dirs.sort();
        self.dirs = dirs;
        let mut files: Vec<(ParsedFile, Option<SystemTime>)> = self
            .md
            .values()
            .map(|entry| (entry.parsed.clone(), entry.mtime))
            .collect();
        files.sort_by(|a, b| a.0.path.cmp(&b.0.path));
        self.corpus = Corpus::assemble(Some(kroot), files);
        self.cites = self
            .corpus
            .nodes
            .iter()
            .map(|node| {
                self.md
                    .get(&node.path)
                    .map(|entry| entry.cites.clone())
                    .unwrap_or_else(|| Arc::new(extract_cites(&node.value)))
            })
            .collect();
    }

    /// KR-NODE-5 walk; reparses a markdown file only when (len, mtime) moved.
    fn walk(&mut self, abs: &Path, rel: &str, seen: &mut HashSet<String>, dirs: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(abs) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(|entry| entry.ok()).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            self.counters.stats += 1;
            if meta.is_dir() {
                dirs.push(child.clone());
                self.walk(&entry.path(), &child, seen, dirs);
                continue;
            }
            if !name.to_ascii_lowercase().ends_with(".md") {
                continue;
            }
            let len = meta.len();
            let mtime = meta.modified().ok();
            if let Some(cached) = self.md.get(&child)
                && cached.len == len
                && cached.mtime == mtime
            {
                seen.insert(child);
                continue;
            }
            let Ok(markdown) = std::fs::read_to_string(entry.path()) else {
                self.md.remove(&child);
                continue;
            };
            self.counters.md_parses += 1;
            let parsed = document::parse_knowledge_markdown(&child, &markdown);
            let cites = Arc::new(extract_cites(&parsed.body));
            seen.insert(child.clone());
            self.md.insert(
                child,
                MdEntry {
                    len,
                    mtime,
                    parsed,
                    cites,
                },
            );
        }
    }

    // ---------------------------------------------------------- files

    fn cited_paths(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for cites in &self.cites {
            for cite in cites.iter() {
                if let Some(path) = cite.norm_path()
                    && is_workspace_file_ref(&path)
                {
                    out.insert(path);
                }
            }
        }
        out
    }

    fn sweep_files(&mut self, hint: &Hint) {
        let cited = self.cited_paths();
        let lowered: Vec<String> = hint.paths.iter().map(|p| p.to_lowercase()).collect();
        let mut listings = Listings::default();
        for path in &cited {
            let due = hint.full
                || !self.files.contains_key(path)
                || hinted(&lowered, &path.to_lowercase());
            if due {
                self.stat_into(path, &mut listings);
            }
        }
    }

    fn stat_into(&mut self, path: &str, listings: &mut Listings) {
        self.counters.stats += 1;
        let stamp = fsref::lookup(&self.workspace, path, listings);
        match self.files.get_mut(path) {
            Some(entry) if entry.stamp == stamp => {}
            Some(entry) => {
                entry.stamp = stamp;
                entry.generation = self.next_generation;
                entry.loaded = None;
                self.next_generation += 1;
            }
            None => {
                self.files.insert(
                    path.to_string(),
                    FileEntry {
                        stamp,
                        generation: self.next_generation,
                        loaded: None,
                    },
                );
                self.next_generation += 1;
            }
        }
    }

    fn ensure_stat(&mut self, path: &str) -> Stamp {
        if !self.files.contains_key(path) {
            self.stat_into(path, &mut Listings::default());
        }
        self.files[path].stamp
    }

    fn loaded(&mut self, path: &str) -> Option<&Loaded> {
        let stamp = self.ensure_stat(path);
        let need = self
            .files
            .get(path)
            .is_some_and(|entry| entry.loaded.is_none());
        if need {
            let loaded = match stamp.kind {
                Kind::Missing | Kind::Escape => None,
                Kind::Dir => Some(Loaded::Unreadable(Unreadable::Directory)),
                Kind::File => {
                    self.counters.file_reads += 1;
                    match fsref::read_text(&self.workspace, path, &mut Listings::default()).1 {
                        TextRead::Text(text) => Some(Loaded::Text(Arc::new(TextEntry::new(text)))),
                        TextRead::Unreadable(why) => Some(Loaded::Unreadable(why)),
                        TextRead::Missing | TextRead::Escape => None,
                    }
                }
            };
            if let Some(entry) = self.files.get_mut(path) {
                entry.loaded = loaded;
            }
        }
        self.files.get(path).and_then(|entry| entry.loaded.as_ref())
    }

    fn text_entry(&mut self, path: &str) -> Option<Arc<TextEntry>> {
        match self.loaded(path) {
            Some(Loaded::Text(text)) => Some(text.clone()),
            _ => None,
        }
    }

    fn prune(&mut self) {
        let cited = self.cited_paths();
        self.files.retain(|path, _| cited.contains(path));
        self.symbols.retain(|(path, _), _| cited.contains(path));
    }

    // ---------------------------------------------------------- symbols

    fn symbol_detail(&mut self, path: &str, chain: &str) -> SymbolDetail {
        let none = |fact| SymbolDetail {
            fact,
            start_line: None,
            end_line: None,
        };
        let Some(text) = self.text_entry(path) else {
            return none(SymbolFact::Missing);
        };
        let generation = self.files.get(path).map(|e| e.generation).unwrap_or(0);
        let cache_key = (path.to_string(), chain.to_string());
        if let Some((gen_at, detail)) = self.symbols.get(&cache_key)
            && *gen_at == generation
        {
            return *detail;
        }
        let mut parses = 0;
        let detail = match text.scopes(path, &mut parses) {
            None => none(SymbolFact::Unsupported),
            Some(scopes) => match match_scopes(&scopes, chain) {
                ScopeMatch::Missing => none(SymbolFact::Missing),
                ScopeMatch::Ambiguous(_) => none(SymbolFact::Ambiguous),
                ScopeMatch::Unique(entry) => SymbolDetail {
                    fact: SymbolFact::Present,
                    start_line: Some(entry.start_line),
                    end_line: Some(entry.end_line),
                },
            },
        };
        self.counters.ts_parses += parses;
        self.symbols.insert(cache_key, (generation, detail));
        detail
    }

    /// `/symbol-refs`: arbitrary (file, chain) queries under the same rules.
    pub fn query_symbols(&mut self, queries: &[(String, Option<String>)]) -> Vec<SymbolQuery> {
        let mut listings = Listings::default();
        let mut restatted = HashSet::new();
        queries
            .iter()
            .map(|(file, symbol)| {
                let path = normalize_ref_path(file);
                let symbol = symbol
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let mut out = SymbolQuery {
                    status: "missing_file",
                    file_exists: false,
                    symbol_exists: false,
                    ambiguous: false,
                    start_line: None,
                    end_line: None,
                };
                if !is_workspace_file_ref(&path) {
                    return out;
                }
                if restatted.insert(path.clone()) {
                    self.stat_into(&path, &mut listings);
                }
                match self.files[&path].stamp.kind {
                    Kind::Missing => return out,
                    Kind::Escape => {
                        out.status = "symlink_escape";
                        return out;
                    }
                    Kind::Dir | Kind::File => out.file_exists = true,
                }
                let Some(chain) = symbol else {
                    out.status = "ok";
                    return out;
                };
                if self.text_entry(&path).is_none() {
                    out.status = "unreadable_file";
                    return out;
                }
                let detail = self.symbol_detail(&path, &chain);
                out.status = match detail.fact {
                    SymbolFact::Unsupported => "unsupported_language",
                    SymbolFact::Missing => "missing_symbol",
                    SymbolFact::Ambiguous => {
                        out.ambiguous = true;
                        "ambiguous_symbol"
                    }
                    SymbolFact::Present | SymbolFact::Checking => "ok",
                };
                if detail.fact == SymbolFact::Present {
                    out.symbol_exists = true;
                    out.start_line = detail.start_line;
                    out.end_line = detail.end_line;
                }
                out
            })
            .collect()
    }

    // ---------------------------------------------------------- content

    fn content(&self, resolution: validate::Resolution, ready: bool) -> Content {
        let corpus = &self.corpus;
        let mut nodes = Vec::with_capacity(corpus.nodes.len());
        let mut all_issues = Vec::new();
        for ((node, issues), refs) in corpus
            .nodes
            .iter()
            .zip(resolution.issues)
            .zip(resolution.refs)
        {
            let severity = node_severity(&issues);
            all_issues.extend(issues.iter().cloned());
            nodes.push(NodeView {
                path: node.path.clone(),
                id: node.id.clone(),
                key: node.key.clone(),
                status: node.status.as_str(),
                invalid_status: node.invalid_status.clone(),
                summary: node.summary.clone(),
                body: node.value.clone(),
                x: node.x,
                y: node.y,
                w: node.w,
                h: node.h,
                extras: node.extras.clone(),
                folder_id: node.folder_id.clone(),
                relations: node.relations.clone(),
                severity,
                issues,
                refs,
            });
        }
        let mut folder_ids: HashSet<String> = HashSet::new();
        for dir in &self.dirs {
            add_folder_chain(&mut folder_ids, dir);
        }
        for node in &corpus.nodes {
            if let Some(folder) = &node.folder_id {
                add_folder_chain(&mut folder_ids, folder);
            }
        }
        let unknown: Vec<UnknownView> = corpus
            .unknown
            .iter()
            .map(|path| {
                let folder_id = path.rsplit_once('/').map(|(dir, _)| dir.to_string());
                if let Some(folder) = &folder_id {
                    add_folder_chain(&mut folder_ids, folder);
                }
                UnknownView {
                    name: path.rsplit('/').next().unwrap_or(path).to_string(),
                    path: path.clone(),
                    folder_id,
                }
            })
            .collect();
        let mut folders: Vec<FolderView> = folder_ids
            .into_iter()
            .map(|id| {
                let (parent_id, name) = match id.rsplit_once('/') {
                    Some((parent, name)) => (Some(parent.to_string()), name.to_string()),
                    None => (None, id.clone()),
                };
                FolderView {
                    id,
                    name,
                    parent_id,
                }
            })
            .collect();
        folders.sort_by(|a, b| a.id.cmp(&b.id));
        let note = if ready {
            status::note_from(corpus, &all_issues)
        } else {
            None
        };
        let root = corpus.root.clone();
        let visibility = match root.as_deref() {
            Some(root::PUBLIC_ROOT) => "public",
            _ => "private",
        };
        Content {
            ready,
            root,
            visibility,
            folders,
            unknown,
            nodes,
            note,
            issues: all_issues,
        }
    }
}

pub(crate) fn checks_of(corpus: &Corpus) -> Vec<CheckNode<'_>> {
    corpus
        .nodes
        .iter()
        .map(|node| CheckNode {
            id: &node.id,
            key: &node.key,
            value: &node.value,
            status: node.status,
            invalid_status: node.invalid_status.as_deref(),
            path: &node.path,
        })
        .collect()
}

fn hinted(lowered_hints: &[String], path_lower: &str) -> bool {
    lowered_hints.iter().any(|hint| {
        hint == path_lower
            || (path_lower.len() > hint.len()
                && path_lower.starts_with(hint.as_str())
                && path_lower.as_bytes()[hint.len()] == b'/')
    })
}

fn touches_knowledge_root(path: &str) -> bool {
    let path = path.replace('\\', "/").to_lowercase();
    [root::PUBLIC_ROOT, root::PRIVATE_ROOT, ".litecode"]
        .iter()
        .any(|root| path == *root || path.starts_with(&format!("{root}/")))
}

fn add_folder_chain(ids: &mut HashSet<String>, dir: &str) {
    let dir = dir.trim_matches('/');
    if dir.is_empty() || dir.to_ascii_lowercase().ends_with(".md") {
        return;
    }
    let parts: Vec<&str> = dir.split('/').collect();
    for i in 1..=parts.len() {
        ids.insert(parts[..i].join("/"));
    }
}

fn node_severity(issues: &[Issue]) -> Option<Severity> {
    if issues.iter().any(|i| i.severity == Severity::Error) {
        Some(Severity::Error)
    } else if issues
        .iter()
        .any(|i| i.severity == Severity::Warning && i.code != "inactive_target")
    {
        Some(Severity::Warning)
    } else {
        None
    }
}

struct EngineFacts<'a> {
    engine: &'a mut Engine,
}

impl Facts for EngineFacts<'_> {
    fn file(&mut self, path: &str) -> FileFact {
        match self.engine.ensure_stat(path).kind {
            Kind::Missing => FileFact::Missing,
            Kind::Escape => FileFact::Escape,
            Kind::Dir => FileFact::Dir,
            Kind::File => FileFact::File,
        }
    }

    fn text(&mut self, path: &str) -> TextFact {
        match self.engine.loaded(path) {
            Some(Loaded::Text(text)) => TextFact::Text {
                line_count: text.line_count,
            },
            Some(Loaded::Unreadable(why)) => TextFact::Unreadable(*why),
            None => TextFact::Unreadable(Unreadable::NotText),
        }
    }

    fn symbol(&mut self, path: &str, chain: &str) -> SymbolFact {
        self.engine.symbol_detail(path, chain).fact
    }
}

struct Pending;

impl Facts for Pending {
    fn file(&mut self, _: &str) -> FileFact {
        FileFact::Checking
    }
    fn text(&mut self, _: &str) -> TextFact {
        TextFact::Checking
    }
    fn symbol(&mut self, _: &str, _: &str) -> SymbolFact {
        SymbolFact::Checking
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolQuery {
    pub status: &'static str,
    pub file_exists: bool,
    pub symbol_exists: bool,
    pub ambiguous: bool,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
}

// ------------------------------------------------------------------ snapshot

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FolderView {
    pub id: String,
    pub name: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UnknownView {
    pub path: String,
    pub name: String,
    pub folder_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeView {
    pub path: String,
    pub id: String,
    pub key: String,
    pub status: &'static str,
    pub invalid_status: Option<String>,
    pub summary: String,
    pub body: String,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub w: Option<f64>,
    pub h: Option<f64>,
    pub extras: Vec<String>,
    pub folder_id: Option<String>,
    pub relations: Vec<String>,
    pub severity: Option<Severity>,
    pub issues: Vec<Issue>,
    pub refs: Vec<RefResult>,
}

/// Everything one version says. Compared whole to decide whether a refresh
/// is a new version.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Content {
    pub ready: bool,
    pub root: Option<String>,
    pub visibility: &'static str,
    pub folders: Vec<FolderView>,
    pub unknown: Vec<UnknownView>,
    pub nodes: Vec<NodeView>,
    #[serde(skip)]
    pub note: Option<Note>,
    #[serde(skip)]
    pub issues: Vec<Issue>,
}

impl Content {
    fn empty() -> Self {
        Self {
            ready: false,
            root: None,
            visibility: "private",
            folders: Vec::new(),
            unknown: Vec::new(),
            nodes: Vec::new(),
            note: None,
            issues: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub version: u64,
    pub epoch: Arc<str>,
    pub content: Arc<Content>,
}

#[derive(Serialize)]
struct Wire<'a> {
    version: u64,
    epoch: &'a str,
    #[serde(flatten)]
    content: &'a Content,
}

impl Snapshot {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(Wire {
            version: self.version,
            epoch: &self.epoch,
            content: &self.content,
        })
        .unwrap_or(serde_json::Value::Null)
    }
}

/// `knowledge/changed` push payload.
#[derive(Debug, Clone, Serialize)]
pub struct Changed {
    pub version: u64,
    pub epoch: Arc<str>,
    /// Node paths whose rendering changed (hint only; refetch the snapshot).
    pub paths: Vec<String>,
}

// ------------------------------------------------------------------ index

#[derive(Default)]
struct Queue {
    hint: Hint,
    queued: u64,
    done: u64,
    stop: bool,
}

pub struct KnowledgeIndex {
    workspace: PathBuf,
    engine: Mutex<Engine>,
    current: RwLock<Arc<Snapshot>>,
    queue: Mutex<Queue>,
    wake: Condvar,
    changed: tokio::sync::broadcast::Sender<Changed>,
    /// Lowercased cited paths, for the change-bus relevance filter.
    cited: RwLock<Arc<HashSet<String>>>,
}

impl KnowledgeIndex {
    fn new(workspace: &Path) -> Arc<Self> {
        let epoch: Arc<str> = format!(
            "{:x}-{:x}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        )
        .into();
        let (changed, _) = tokio::sync::broadcast::channel(64);
        Arc::new(Self {
            workspace: workspace.to_path_buf(),
            engine: Mutex::new(Engine::new(workspace)),
            current: RwLock::new(Arc::new(Snapshot {
                version: 0,
                epoch,
                content: Arc::new(Content::empty()),
            })),
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            changed,
            cited: RwLock::new(Arc::new(HashSet::new())),
        })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Latest published snapshot. Never blocks on a recompute.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Changed> {
        self.changed.subscribe()
    }

    /// Feed workspace-relative paths from the change bus. Paths that cannot
    /// affect the knowledge state are dropped here, so unrelated edits never
    /// make a turn wait.
    pub fn notify_paths(&self, paths: &[String]) {
        let cited = self.cited.read().expect("cited lock").clone();
        let mut hint = Hint::default();
        for path in paths {
            let path = path.replace('\\', "/");
            let lower = path.to_lowercase();
            if touches_knowledge_root(&path)
                || cited.contains(&lower)
                || cited
                    .iter()
                    .any(|c| hinted(std::slice::from_ref(&lower), c))
            {
                hint.paths.insert(path);
            }
        }
        if !hint.is_empty() {
            self.enqueue(hint);
        }
    }

    /// The bus dropped events: treat as a full sweep.
    pub fn notify_lagged(&self) {
        self.enqueue(Hint::full());
    }

    fn enqueue(&self, hint: Hint) {
        let mut queue = self.queue.lock().expect("knowledge queue");
        queue.hint.merge(hint);
        queue.queued += 1;
        self.wake.notify_all();
    }

    /// Wait (at most `max`) until everything queued before this call is
    /// published. `true` when nothing was left pending.
    pub fn wait_fresh(&self, max: Duration) -> bool {
        let deadline = Instant::now() + max;
        let mut queue = self.queue.lock().expect("knowledge queue");
        let target = queue.queued;
        while queue.done < target {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            queue = self
                .wake
                .wait_timeout(queue, deadline - now)
                .expect("knowledge queue")
                .0;
        }
        true
    }

    /// Synchronous full refresh (tool path): fresh after the caller's writes.
    pub fn refresh_now(&self) -> Arc<Snapshot> {
        let content = self
            .engine
            .lock()
            .expect("knowledge engine")
            .refresh(&Hint::full());
        self.publish(content)
    }

    pub fn with_engine<T>(&self, f: impl FnOnce(&mut Engine) -> T) -> T {
        f(&mut self.engine.lock().expect("knowledge engine"))
    }

    fn publish(&self, content: Content) -> Arc<Snapshot> {
        let cited: HashSet<String> = content
            .nodes
            .iter()
            .flat_map(|node| node.refs.iter())
            .filter_map(|r| r.path.as_ref())
            .map(|p| p.to_lowercase())
            .collect();
        *self.cited.write().expect("cited lock") = Arc::new(cited);
        let mut current = self.current.write().expect("knowledge snapshot lock");
        if *current.content == content {
            return current.clone();
        }
        let paths = changed_node_paths(&current.content, &content);
        let next = Arc::new(Snapshot {
            version: current.version + 1,
            epoch: current.epoch.clone(),
            content: Arc::new(content),
        });
        *current = next.clone();
        drop(current);
        let _ = self.changed.send(Changed {
            version: next.version,
            epoch: next.epoch.clone(),
            paths,
        });
        next
    }

    fn run(self: Arc<Self>) {
        {
            let _lap = latency::lap("knowledge_index.preview");
            let content = self.engine.lock().expect("knowledge engine").preview();
            self.publish(content);
        }
        {
            let _lap = latency::lap("knowledge_index.build");
            self.refresh_now();
        }
        loop {
            let (hint, target) = {
                let mut queue = self.queue.lock().expect("knowledge queue");
                let deadline = Instant::now() + SWEEP_EVERY;
                while queue.queued == queue.done && !queue.stop {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    queue = self
                        .wake
                        .wait_timeout(queue, deadline - now)
                        .expect("knowledge queue")
                        .0;
                }
                if queue.stop {
                    return;
                }
                if queue.queued == queue.done {
                    (Hint::full(), queue.done)
                } else {
                    (std::mem::take(&mut queue.hint), queue.queued)
                }
            };
            let content = self.engine.lock().expect("knowledge engine").refresh(&hint);
            self.publish(content);
            let mut queue = self.queue.lock().expect("knowledge queue");
            queue.done = queue.done.max(target);
            self.wake.notify_all();
        }
    }

    /// Stop the worker (tests).
    pub fn stop(&self) {
        let mut queue = self.queue.lock().expect("knowledge queue");
        queue.stop = true;
        self.wake.notify_all();
    }
}

fn changed_node_paths(old: &Content, new: &Content) -> Vec<String> {
    let before: HashMap<&str, &NodeView> = old.nodes.iter().map(|n| (n.path.as_str(), n)).collect();
    let after: HashMap<&str, &NodeView> = new.nodes.iter().map(|n| (n.path.as_str(), n)).collect();
    let mut paths: Vec<String> = after
        .iter()
        .filter(|(path, node)| before.get(*path) != Some(*node))
        .map(|(path, _)| path.to_string())
        .chain(
            before
                .keys()
                .filter(|path| !after.contains_key(*path))
                .map(|path| path.to_string()),
        )
        .collect();
    paths.sort();
    paths
}

// ------------------------------------------------------------------ registry

static REGISTRY: LazyLock<Mutex<HashMap<PathBuf, Arc<KnowledgeIndex>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Start (once) the resident index for `workspace` and its worker thread.
/// Returns immediately; the first snapshots are `ready: false`.
pub fn start(workspace: &Path) -> Arc<KnowledgeIndex> {
    let canon = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    let mut registry = REGISTRY.lock().expect("knowledge registry");
    if let Some(index) = registry.get(&canon) {
        let index = index.clone();
        registry.insert(workspace.to_path_buf(), index.clone());
        return index;
    }
    let index = KnowledgeIndex::new(workspace);
    registry.insert(canon, index.clone());
    registry.insert(workspace.to_path_buf(), index.clone());
    drop(registry);
    let worker = index.clone();
    if let Err(error) = std::thread::Builder::new()
        .name("knowledge-index".into())
        .spawn(move || worker.run())
    {
        tracing::warn!(%error, "knowledge index worker did not start");
    }
    index
}

/// The resident index for `workspace`, when one was started.
pub fn registered(workspace: &Path) -> Option<Arc<KnowledgeIndex>> {
    let mut registry = REGISTRY.lock().expect("knowledge registry");
    if let Some(index) = registry.get(workspace) {
        return Some(index.clone());
    }
    if registry.is_empty() {
        return None;
    }
    let canon = std::fs::canonicalize(workspace).ok()?;
    let index = registry.get(&canon)?.clone();
    // Remember this spelling so the next lookup is a map hit.
    registry.insert(workspace.to_path_buf(), index.clone());
    Some(index)
}

/// Remove an index (tests).
#[cfg(test)]
pub fn unregister(workspace: &Path) {
    let mut registry = REGISTRY.lock().expect("knowledge registry");
    let target = registry.get(workspace).cloned();
    if let Some(target) = target {
        target.stop();
        registry.retain(|_, index| !Arc::ptr_eq(index, &target));
    }
}

/// From-scratch content for a workspace with no resident index (CLI, tests).
pub fn compute_once(workspace: &Path) -> Content {
    Engine::new(workspace).refresh(&Hint::full())
}

/// Fresh content for a reader that must see its own writes (the
/// `knowledge` tool): the resident index refreshed now, or a one-shot pass.
pub fn fresh_content(workspace: &Path) -> Arc<Content> {
    match registered(workspace) {
        Some(index) => index.refresh_now().content.clone(),
        None => Arc::new(compute_once(workspace)),
    }
}

/// The knowledge status note for a turn: snapshot read only.
///
/// With a resident index: wait at most [`TURN_WAIT`] for queued recomputes,
/// then read the latest complete snapshot; `None` while it is not ready
/// (the first turn simply skips the note). Without one (CLI / tests), one
/// from-scratch pass.
pub fn turn_note(workspace: &Path) -> Option<Note> {
    let mut stages = latency::Stages::new("knowledge_status.note", None);
    let Some(index) = registered(workspace) else {
        let content = compute_once(workspace);
        stages.mark("oneshot");
        return content.note;
    };
    stages.mark("lookup");
    let fresh = index.wait_fresh(TURN_WAIT);
    stages.mark("wait_fresh");
    let snapshot = index.snapshot();
    stages.mark("snapshot_read");
    stages.note("fresh", fresh);
    stages.note("ready", snapshot.content.ready);
    stages.note("version", snapshot.version);
    if !snapshot.content.ready {
        return None;
    }
    snapshot.content.note.clone()
}

/// `/symbol-refs` through the same engine (and caches) as the index.
pub fn query_symbols(workspace: &Path, queries: &[(String, Option<String>)]) -> Vec<SymbolQuery> {
    match registered(workspace) {
        Some(index) => index.with_engine(|engine| engine.query_symbols(queries)),
        None => Engine::new(workspace).query_symbols(queries),
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
