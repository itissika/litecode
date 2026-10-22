//! Single outbound encoder for client-facing `Item` payloads.
//!
//! All wire paths that ship a full committed item (`buffer/load`, `buffer/item`)
//! must pass through this module so blob resolution and serde shape stay identical.

use std::path::Path;

use crate::tool::output;
use crate::types::{Item, Result, Transcript};

/// Resolve spilled blob refs and return items ready for client wire serde.
pub fn encode_client_items(mut items: Transcript, data_root: &Path) -> Result<Transcript> {
    output::resolve_body_refs(&mut items, data_root)?;
    Ok(items)
}

/// Encode a single item for client wire serde.
pub fn encode_client_item(mut item: Item, data_root: &Path) -> Result<Item> {
    let mut items = vec![item];
    output::resolve_body_refs(&mut items, data_root)?;
    item = items.remove(0);
    Ok(item)
}
