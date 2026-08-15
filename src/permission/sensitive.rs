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
mod tests {
    use super::*;

    #[test]
    fn detects_unix_and_windows_system_paths() {
        assert!(is_sensitive_system_path("/etc/passwd"));
        assert!(is_sensitive_system_path("/boot/vmlinuz"));
        assert!(is_sensitive_system_path("/usr/bin/python3"));
        assert!(is_sensitive_system_path(r"C:\Windows\System32\drivers"));
        assert!(is_sensitive_system_path(r"C:\Program Files\Git\etc\passwd"));
        assert!(!is_sensitive_system_path("/home/user/project/main.rs"));
        assert!(!is_sensitive_system_path("/tmp/test.txt"));
    }
}
