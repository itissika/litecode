//! On-disk knowledge corpus. Files are the only source of truth; nothing here
//! is indexed or cached across tool calls.

pub mod corpus;
pub mod document;
pub mod mentions;
pub mod reminder;
pub mod root;
pub mod status;
pub mod symbol_check;
pub mod validate;
pub mod view;

pub use corpus::{Corpus, Node};
pub use document::{ParsedFile, Status};
pub use validate::{Issue, Severity};
