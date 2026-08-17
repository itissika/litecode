//! Resolve the compile-time release channel for `server/hello`.
//!
//! `LITECODE_CHANNEL` is read here (not in source) so Cargo invalidates the build
//! when the channel changes between dev / nightly / official artifacts.

fn main() {
    println!("cargo:rerun-if-env-changed=LITECODE_CHANNEL");

    let channel = match std::env::var("LITECODE_CHANNEL")
        .ok()
        .as_deref()
        .map(str::trim)
    {
        Some("nightly") => "nightly",
        Some("official") => "official",
        Some("dev") => "dev",
        _ => "dev",
    };

    println!("cargo:rustc-env=LITECODE_BUILD_CHANNEL={channel}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_channel_is_dev() {
        let channel = match std::env::var("LITECODE_CHANNEL")
            .ok()
            .as_deref()
            .map(str::trim)
        {
            Some("nightly") => "nightly",
            Some("official") => "official",
            Some("dev") => "dev",
            _ => "dev",
        };
        // build.rs logic mirror — unset/unknown env resolves to dev.
        if std::env::var("LITECODE_CHANNEL").is_err() {
            assert_eq!(channel, "dev");
        }
    }
}
