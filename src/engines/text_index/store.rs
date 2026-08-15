//! Tantivy ngram store for adaptive text index.

use grep::regex::RegexMatcherBuilder;
use grep::searcher::{
    BinaryDetection, SearcherBuilder, Sink, SinkContext, SinkContextKind, SinkMatch,
};
use std::collections::HashSet;
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, STRING, Schema, TantivyDocument, TextFieldIndexing,
    TextOptions, Value,
};
use tantivy::tokenizer::NgramTokenizer;
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, Term, doc};

use crate::engines::code_search::{LexicalMatch, LexicalQuery, LexicalSearchOutcome};
use crate::types::{LitecodeError, Result};
use crate::workspace::filter::{
    FilterPreset, RelPathCtx, cheap_rel_under, compile_include_patterns, looks_binary,
    path_matches_include, walk_builder,
};

use super::literal::{indexable_literal, trigrams};
use super::meta::tantivy_dir;
use super::policy::MAX_INDEX_FILE_BYTES;

const WRITER_HEAP_BYTES: usize = 50_000_000;
const TOKENIZER_NAME: &str = "ngram3_lc";
const MAX_CANDIDATES: usize = 2_000;

struct Fields {
    path: Field,
    body: Field,
}

fn build_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let body_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(TOKENIZER_NAME)
            .set_index_option(IndexRecordOption::Basic),
    );
    let path = builder.add_text_field("path", STRING | STORED);
    let body = builder.add_text_field("body", body_opts);
    let schema = builder.build();
    (schema, Fields { path, body })
}

fn register_tokenizers(index: &Index) {
    let ngram = NgramTokenizer::new(3, 3, false).expect("ngram3");
    index.tokenizers().register(TOKENIZER_NAME, ngram);
}

pub struct TextIndexStore {
    #[allow(dead_code)]
    index: Index,
    reader: IndexReader,
    fields: Fields,
}

impl TextIndexStore {
    pub fn clone_reader(&self) -> Result<Self> {
        let reader = self
            .index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e| LitecodeError::Config(format!("text_index reader: {e}")))?;
        Ok(Self {
            index: self.index.clone(),
            reader,
            fields: Fields {
                path: self.fields.path,
                body: self.fields.body,
            },
        })
    }

    pub fn open(workspace_root: &Path) -> Result<Self> {
        let dir = tantivy_dir(workspace_root);
        if !dir.is_dir() {
            return Err(LitecodeError::Config(format!(
                "text_index missing at {}",
                dir.display()
            )));
        }
        let index = Index::open_in_dir(&dir)
            .map_err(|e| LitecodeError::Config(format!("text_index open: {e}")))?;
        register_tokenizers(&index);
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e| LitecodeError::Config(format!("text_index reader: {e}")))?;
        let schema = index.schema();
        let fields = Fields {
            path: schema
                .get_field("path")
                .map_err(|_| LitecodeError::Config("text_index schema path".into()))?,
            body: schema
                .get_field("body")
                .map_err(|_| LitecodeError::Config("text_index schema body".into()))?,
        };
        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    pub fn build(workspace_root: &Path, should_stop: impl Fn() -> bool) -> Result<Self> {
        let dir = tantivy_dir(workspace_root);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
        }
        std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;

        let (schema, fields) = build_schema();
        let index = Index::create_in_dir(&dir, schema)
            .map_err(|e| LitecodeError::Config(format!("text_index create: {e}")))?;
        register_tokenizers(&index);
        let mut writer: IndexWriter = index
            .writer(WRITER_HEAP_BYTES)
            .map_err(|e| LitecodeError::Config(format!("text_index writer: {e}")))?;

        let rel_ctx = RelPathCtx::new(workspace_root)
            .unwrap_or_else(|_| RelPathCtx::new_lossy(workspace_root));
        let mut added = 0u64;
        for entry in walk_builder(workspace_root, FilterPreset::AgentText).build() {
            if should_stop() {
                break;
            }
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let path = entry.path();
            let Some(rel) = cheap_rel_under(workspace_root, path).or_else(|| rel_ctx.rel(path))
            else {
                continue;
            };
            if let Some(content) = read_indexable_file(path) {
                let content = content.to_lowercase();
                writer
                    .add_document(doc!(
                        fields.path => rel,
                        fields.body => content,
                    ))
                    .map_err(|e| LitecodeError::Config(format!("text_index add: {e}")))?;
                added += 1;
                if added % 5000 == 0 {
                    tracing::debug!(added, "text_index build progress");
                }
            }
        }
        writer
            .commit()
            .map_err(|e| LitecodeError::Config(format!("text_index commit: {e}")))?;

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e| LitecodeError::Config(format!("text_index reader: {e}")))?;
        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    pub fn apply_updates(
        &mut self,
        workspace_root: &Path,
        updates: &[(String, bool)],
    ) -> Result<()> {
        let mut writer: IndexWriter = self
            .index
            .writer(WRITER_HEAP_BYTES)
            .map_err(|e| LitecodeError::Config(format!("text_index writer: {e}")))?;
        for (rel, deleted) in updates {
            let term = Term::from_field_text(self.fields.path, rel);
            writer.delete_term(term);
            if *deleted {
                continue;
            }
            let abs = workspace_root.join(rel);
            if let Some(content) = read_indexable_file(&abs) {
                let content = content.to_lowercase();
                writer
                    .add_document(doc!(
                        self.fields.path => rel.as_str(),
                        self.fields.body => content,
                    ))
                    .map_err(|e| LitecodeError::Config(format!("text_index add: {e}")))?;
            }
        }
        writer
            .commit()
            .map_err(|e| LitecodeError::Config(format!("text_index commit: {e}")))?;
        self.reader
            .reload()
            .map_err(|e| LitecodeError::Config(format!("text_index reload: {e}")))?;
        Ok(())
    }

    /// Returns candidate relative paths, or `None` if the query is not indexable.
    pub fn search_candidates(&self, query: &LexicalQuery) -> Result<Option<Vec<String>>> {
        let Some(lit) = indexable_literal(&query.pattern, query.is_regex) else {
            return Ok(None);
        };
        let grams = trigrams(&lit);
        if grams.is_empty() {
            return Ok(None);
        }

        let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        for g in &grams {
            let term = Term::from_field_text(self.fields.body, g);
            clauses.push((
                Occur::Must,
                Box::new(TermQuery::new(term, IndexRecordOption::Basic)),
            ));
        }
        // Optional path prefix when searching a subdirectory.
        if let Some(sub) = query.path.as_ref() {
            let prefix = path_prefix_under(&query.root, sub);
            if let Some(pref) = prefix {
                // Soft filter after retrieval; stored for verify scope.
                let _ = pref;
            }
        }

        let bq = BooleanQuery::new(clauses);
        let searcher = self.reader.searcher();
        let top = searcher
            .search(&bq, &TopDocs::with_limit(MAX_CANDIDATES))
            .map_err(|e| LitecodeError::Config(format!("text_index search: {e}")))?;

        let mut paths = Vec::new();
        let mut seen = HashSet::new();
        let path_prefix = query
            .path
            .as_ref()
            .and_then(|p| path_prefix_under(&query.root, p));
        for (_score, addr) in top {
            let doc: TantivyDocument = searcher
                .doc(addr)
                .map_err(|e| LitecodeError::Config(format!("text_index doc: {e}")))?;
            let Some(path_val) = doc.get_first(self.fields.path) else {
                continue;
            };
            let Some(path) = path_val.as_str() else {
                continue;
            };
            if let Some(ref pref) = path_prefix
                && !path.starts_with(pref.as_str())
                && path != pref.as_str()
            {
                continue;
            }
            if seen.insert(path.to_string()) {
                paths.push(path.to_string());
            }
        }
        Ok(Some(paths))
    }
}

fn path_prefix_under(workspace_root: &Path, search_path: &Path) -> Option<String> {
    let abs = if search_path.is_absolute() {
        search_path.to_path_buf()
    } else {
        workspace_root.join(search_path)
    };
    let rel = cheap_rel_under(workspace_root, &abs)?;
    if rel.is_empty() {
        None
    } else {
        Some(rel.replace('\\', "/"))
    }
}

fn read_indexable_file(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_INDEX_FILE_BYTES {
        return None;
    }
    if looks_binary(path) {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Count files under AgentText walk, stopping after `limit` (inclusive stop).
pub fn count_agent_text_files(workspace_root: &Path, limit: u64) -> Result<u64> {
    let mut n = 0u64;
    for entry in walk_builder(workspace_root, FilterPreset::AgentText).build() {
        let Ok(entry) = entry else {
            continue;
        };
        if entry.file_type().is_some_and(|t| t.is_file()) {
            n += 1;
            if n >= limit {
                break;
            }
        }
    }
    Ok(n)
}

/// Verify candidates with the same ripgrep matcher as LexicalLane.
pub fn verify_with_ripgrep(
    workspace_root: &Path,
    query: &LexicalQuery,
    _preset: FilterPreset,
    candidates: &[String],
) -> Result<LexicalSearchOutcome> {
    let pattern = if query.is_regex {
        query.pattern.clone()
    } else {
        regex::escape(&query.pattern)
    };
    let matcher = RegexMatcherBuilder::new()
        .case_insensitive(!query.case_sensitive)
        .word(query.whole_word)
        .multi_line(query.multiline)
        .dot_matches_new_line(query.multiline)
        .build(&pattern)
        .map_err(|e| LitecodeError::ToolExecution(format!("invalid search pattern: {e}")))?;

    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .multi_line(query.multiline)
        .before_context(query.before_context)
        .after_context(query.after_context)
        .binary_detection(BinaryDetection::quit(b'\0'))
        .build();

    let include = match query.include.as_deref() {
        None | Some("") => Vec::new(),
        Some(p) => compile_include_patterns(p)?,
    };

    let mut matches = Vec::new();
    let mut files_searched = 0usize;
    for rel in candidates {
        if matches.len() >= query.max_matches {
            break;
        }
        if !include.is_empty() && !path_matches_include(rel, &include) {
            continue;
        }
        let abs = workspace_root.join(rel);
        if !abs.is_file() {
            continue;
        }
        files_searched += 1;
        let mut sink = MatchSink {
            path: rel.clone(),
            max: query.max_matches,
            out: &mut matches,
            before: Vec::new(),
            after_budget: query.after_context,
            pending_after: 0,
        };
        let _ = searcher.search_path(&matcher, &abs, &mut sink);
    }
    Ok(LexicalSearchOutcome {
        matches,
        files_searched,
    })
}

struct MatchSink<'a> {
    path: String,
    max: usize,
    out: &'a mut Vec<LexicalMatch>,
    before: Vec<String>,
    after_budget: usize,
    pending_after: usize,
}

impl Sink for MatchSink<'_> {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _searcher: &grep::searcher::Searcher,
        mat: &SinkMatch<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        if self.out.len() >= self.max {
            return Ok(false);
        }
        let line_text = String::from_utf8_lossy(mat.bytes()).into_owned();
        let start_line = mat.line_number().unwrap_or(1) as u32;
        self.out.push(LexicalMatch {
            path: self.path.clone(),
            start_line,
            end_line: start_line,
            line_text,
            context_before: self.before.clone(),
            context_after: Vec::new(),
        });
        self.pending_after = self.after_budget;
        self.before.clear();
        Ok(self.out.len() < self.max)
    }

    fn context(
        &mut self,
        _searcher: &grep::searcher::Searcher,
        ctx: &SinkContext<'_>,
    ) -> std::result::Result<bool, Self::Error> {
        let text = String::from_utf8_lossy(ctx.bytes()).into_owned();
        match ctx.kind() {
            SinkContextKind::Before => {
                self.before.push(text);
            }
            SinkContextKind::After => {
                if self.pending_after > 0
                    && let Some(last) = self.out.last_mut()
                {
                    last.context_after.push(text);
                    self.pending_after -= 1;
                }
            }
            _ => {}
        }
        Ok(true)
    }
}
