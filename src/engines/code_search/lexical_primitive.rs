//! L1 LexicalPrimitive — stable in-process text search API.
//!
//! Consumers: agent `grep`, human text column, and (future) SemanticEngine borrow.
//! Implementation is the LexicalLane (`lexical_search`); this type is the only
//! intended call site for new code.

use crate::types::Result;

use super::lexical::{LexicalMatch, LexicalQuery, lexical_search};

/// Borrowable lexical retrieval primitive (no Warm / no index).
#[derive(Debug, Default, Clone, Copy)]
pub struct LexicalPrimitive;

impl LexicalPrimitive {
    pub fn search(query: &LexicalQuery) -> Result<Vec<LexicalMatch>> {
        lexical_search(query)
    }
}

#[cfg(test)]
mod tests {
    
    
    

}
