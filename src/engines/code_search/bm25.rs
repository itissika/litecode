//! Tantivy BM25 over chunk text + path (Code-corpus sparse leg).
//!
//! Production Code semantic uses Bruch CC (α on dense); RRF remains eval-only.
//! This sidecar is owned by [`super::CodeSearchRuntime`] for workspace Code
//! only — Session search must not share this index or its fusion α.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, STORED, Schema, TantivyDocument, TextFieldIndexing, TextOptions,
    Value,
};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, Term, doc};

use crate::types::{LitecodeError, Result};

use super::chunk::ChunkRecord;
use super::code_tokenize::{expand_for_index, extract_identifiers, query_terms};
use super::meta::index_dir;

const WRITER_HEAP_BYTES: usize = 50_000_000;
const TOKENIZER: &str = "whitespace";

pub fn bm25_dir(workspace_root: &Path) -> PathBuf {
    index_dir(workspace_root).join("bm25")
}

struct Fields {
    chunk_id: Field,
    path: Field,
    body: Field,
}

fn build_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let text_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    let path_opts = text_opts.clone().set_stored();
    let body_opts = text_opts;
    let chunk_id = builder.add_u64_field("chunk_id", STORED);
    let path = builder.add_text_field("path", path_opts);
    let body = builder.add_text_field("body", body_opts);
    let schema = builder.build();
    (
        schema,
        Fields {
            chunk_id,
            path,
            body,
        },
    )
}

/// Rebuild BM25 index from in-memory chunks (overwrite `index/bm25`).
pub fn rebuild(workspace_root: &Path, chunks: &HashMap<u64, ChunkRecord>) -> Result<()> {
    let dir = bm25_dir(workspace_root);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| LitecodeError::Config(e.to_string()))?;

    let (schema, fields) = build_schema();
    let index = Index::create_in_dir(&dir, schema)
        .map_err(|e| LitecodeError::Config(format!("tantivy create: {e}")))?;
    let mut writer: IndexWriter = index
        .writer(WRITER_HEAP_BYTES)
        .map_err(|e| LitecodeError::Config(format!("tantivy writer: {e}")))?;

    let mut ids: Vec<u64> = chunks.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let chunk = &chunks[&id];
        let path_expanded = expand_for_index(&chunk.path.replace(['/', '\\', '-', '.'], " "));
        let body_expanded = expand_for_index(&chunk.text);
        writer
            .add_document(doc!(
                fields.chunk_id => id,
                fields.path => path_expanded,
                fields.body => body_expanded,
            ))
            .map_err(|e| LitecodeError::Config(format!("tantivy add_document: {e}")))?;
    }
    writer
        .commit()
        .map_err(|e| LitecodeError::Config(format!("tantivy commit: {e}")))?;
    Ok(())
}

pub struct Bm25Index {
    #[allow(dead_code)]
    index: Index,
    reader: IndexReader,
    fields: Fields,
}

impl Bm25Index {
    pub fn open(workspace_root: &Path) -> Result<Self> {
        let dir = bm25_dir(workspace_root);
        if !dir.is_dir() {
            return Err(LitecodeError::Config(format!(
                "bm25 index missing at {}",
                dir.display()
            )));
        }
        let index = Index::open_in_dir(&dir)
            .map_err(|e| LitecodeError::Config(format!("tantivy open: {e}")))?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e| LitecodeError::Config(format!("tantivy reader: {e}")))?;
        let schema = index.schema();
        let fields = Fields {
            chunk_id: schema
                .get_field("chunk_id")
                .map_err(|_| LitecodeError::Config("bm25 schema missing chunk_id".into()))?,
            path: schema
                .get_field("path")
                .map_err(|_| LitecodeError::Config("bm25 schema missing path".into()))?,
            body: schema
                .get_field("body")
                .map_err(|_| LitecodeError::Config("bm25 schema missing body".into()))?,
        };
        Ok(Self {
            index,
            reader,
            fields,
        })
    }

    #[allow(dead_code)] // retained for eval-style open-if-present / rebuild-if-missing
    pub fn open_or_rebuild(
        workspace_root: &Path,
        chunks: &HashMap<u64, ChunkRecord>,
    ) -> Result<Self> {
        if !bm25_dir(workspace_root).is_dir() {
            rebuild(workspace_root, chunks)?;
        }
        Self::open(workspace_root)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<(u64, f32)>> {
        let limit = limit.max(1);
        let q = build_query(&self.fields, query);
        let searcher = self.reader.searcher();
        let top: Vec<(f32, tantivy::DocAddress)> = searcher
            .search(&*q, &TopDocs::with_limit(limit))
            .map_err(|e| LitecodeError::Config(format!("tantivy search: {e}")))?;

        let mut out = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let doc: TantivyDocument = searcher
                .doc(addr)
                .map_err(|e| LitecodeError::Config(format!("tantivy doc: {e}")))?;
            let Some(id_val) = doc.get_first(self.fields.chunk_id) else {
                continue;
            };
            let Some(id) = id_val.as_u64() else {
                continue;
            };
            out.push((id, score));
        }
        Ok(out)
    }
}

fn build_query(fields: &Fields, query: &str) -> Box<dyn Query> {
    let mut tokens = query_terms(query);
    // Cody-style: ensure identifier-shaped tokens from the query are present.
    for id in extract_identifiers(query) {
        if !tokens.iter().any(|t| t == &id) {
            tokens.push(id);
        }
    }
    if tokens.is_empty() {
        return Box::new(BooleanQuery::from(Vec::<(Occur, Box<dyn Query>)>::new()));
    }
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    for tok in tokens {
        let body_term = Term::from_field_text(fields.body, &tok);
        let path_term = Term::from_field_text(fields.path, &tok);
        clauses.push((
            Occur::Should,
            Box::new(TermQuery::new(body_term, IndexRecordOption::WithFreqs)),
        ));
        clauses.push((
            Occur::Should,
            Box::new(TermQuery::new(path_term, IndexRecordOption::WithFreqs)),
        ));
    }
    Box::new(BooleanQuery::from(clauses))
}
