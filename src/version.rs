//! Build-time version metadata surfaced in `server/hello`.
//!
//! Channels (set via `LITECODE_CHANNEL` at compile time — see root `build.rs`):
//! - `dev` — script startup (`serve.sh`, `cargo run`, …)
//! - `nightly` — local product builds (`assemble_product`, `package_*`)
//! - `official` — release CI / signed installers (internal; UI hides the channel tag)

/// Application semver from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Release channel baked into the binary at compile time.
pub fn channel() -> &'static str {
    env!("LITECODE_BUILD_CHANNEL")
}
