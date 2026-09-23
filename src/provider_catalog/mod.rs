//! Declarative provider catalog - the single source of truth for LLM providers
//! and models.
//!
//! `provider-catalog.toml` states endpoints, protocols, auth modes, context
//! windows, modalities and reasoning mappings. Rust owns exactly two things:
//! the catalog contract (this module) and one codec per
//! [EndpointKind](schema::EndpointKind). Adding a provider that speaks an
//! existing protocol is a TOML edit plus a restart.

pub mod resolve;
pub mod schema;
pub mod store;

pub use resolve::{ProviderCatalog, ResolvedModel, ResolvedProvider, is_valid_provider_id};
pub use schema::{
    AuthKind, EndpointKind, Modality, ProviderQuirk, ReasoningKey, ReasoningTiers, UsagePatch,
};
pub use store::{
    CATALOG_FILE_NAME, CATALOG_SCHEMA, DEFAULT_CATALOG, INITIALIZED_MARKER, SCHEMA_FILE_NAME,
    catalog_path_for_db, forget, load_for_db, read_and_parse, schema_path_for_db, shared_for_db,
};

#[cfg(test)]
mod tests;
