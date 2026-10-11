//! On-disk knowledge corpus. Files on disk are the only source of truth; the
//! resident [`index::KnowledgeIndex`] caches what was read from them and is
//! the single place every reader gets resolution results from.

pub mod corpus;
pub mod document;
pub mod fsref;
pub mod index;
pub mod mentions;
pub mod reminder;
pub mod root;
pub mod status;
pub mod validate;
pub mod view;

#[cfg(test)]
mod spec_tests;

pub use corpus::{Corpus, Node};
pub use document::{ParsedFile, Status};
pub use validate::{Issue, Severity};
