//! Local web app definitions, registry, install, and runtime.

pub mod catalog;
pub mod install;
pub mod readme;
pub mod registry;
pub mod routes;
pub mod runtime;
pub mod schema;
pub mod service;
pub mod space;
pub mod trust;

pub use registry::{AppRegistry, CatalogApp, RegistrySnapshot};
pub use schema::{
    AppCategory, AppDefinition, AppSource, AppStatus, DataScope, InstallScope, LaunchSpec,
    Permission,
};
pub use service::AppService;
