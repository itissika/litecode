//! Local web app definitions, registry, install, and runtime.

pub mod builtin;
pub mod catalog;
pub mod exec;
pub mod install;
pub mod recommend;
pub mod registry;
pub mod routes;
pub mod runtime;
pub mod schema;
pub mod service;
pub mod trust;

pub use registry::{AppRegistry, RegistrySnapshot};
pub use schema::{AppCategory, AppDefinition, AppSource, AppStatus, LaunchSpec, Permission};
pub use service::AppService;
