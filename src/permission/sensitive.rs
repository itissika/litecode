//! Sensitive OS path heuristics for product floor rules and tool risk warnings.

/// True for paths that look like protected OS locations (raw or resolved strings).
pub fn is_sensitive_system_path(path: &str) -> bool {
    let sensitive_prefixes = [
        "/etc/",
        "/boot/",
        "/sys/",
        "/proc/",
        "/dev/",
        "/usr/lib/",
        "/usr/bin/",
        "/bin/",
        "/sbin/",
        "C:\\Windows\\",
        "C:\\Program Files\\",
        "C:\\ProgramData\\",
    ];
    let normalized = path.replace('\\', "/");
    for prefix in &sensitive_prefixes {
        let normalized_prefix = prefix.replace('\\', "/");
        if normalized.starts_with(&normalized_prefix) {
            return true;
        }
    }
    // Windows Git-mapped `/etc/...` → `...\Git\etc\...`
    normalized.contains("/etc/")
}

#[cfg(test)]
mod tests {}
