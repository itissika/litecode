use std::path::{Path, PathBuf};

fn is_valid_web_dist(path: &Path) -> bool {
    path.is_dir() && path.join("index.html").is_file()
}

/// Resolve the built frontend directory (`web/dist`).
///
/// Search order:
/// 1. `LITECODE_WEB_DIST` env (must exist as a directory with `index.html`)
/// 2. Walk up from `current_exe()` looking for `web/dist` with `index.html`
/// 3. cwd-relative `web/dist`
pub fn resolve_web_dist() -> anyhow::Result<PathBuf> {
    if let Ok(env_path) = std::env::var("LITECODE_WEB_DIST") {
        let path = PathBuf::from(env_path);
        if is_valid_web_dist(&path) {
            return Ok(path);
        }
        anyhow::bail!(
            "LITECODE_WEB_DIST={} is not a valid web dist directory (expected directory with index.html)",
            path.display()
        );
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent();
        while let Some(d) = dir {
            let candidate = d.join("web/dist");
            if is_valid_web_dist(&candidate) {
                return Ok(candidate);
            }
            dir = d.parent();
        }
    }

    let candidate = PathBuf::from("web/dist");
    if is_valid_web_dist(&candidate) {
        return Ok(candidate);
    }

    anyhow::bail!(
        "web dist not found: set LITECODE_WEB_DIST or run `npm run build` in web/ to produce web/dist/index.html"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvRestore {
        _lock: MutexGuard<'static, ()>,
        web_dist: Option<String>,
    }

    impl EnvRestore {
        fn new() -> Self {
            Self {
                _lock: ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()),
                web_dist: std::env::var("LITECODE_WEB_DIST").ok(),
            }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match &self.web_dist {
                Some(value) => unsafe { std::env::set_var("LITECODE_WEB_DIST", value) },
                None => unsafe { std::env::remove_var("LITECODE_WEB_DIST") },
            }
        }
    }

    #[test]
    fn is_valid_web_dist_checks_index_html() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!is_valid_web_dist(dir.path()));
        std::fs::write(dir.path().join("index.html"), "<html></html>").expect("write");
        assert!(is_valid_web_dist(dir.path()));
    }

    #[test]
    fn resolves_env_override() {
        let _restore = EnvRestore::new();
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("index.html"), "<html></html>").expect("write");
        unsafe { std::env::set_var("LITECODE_WEB_DIST", dir.path()) };

        let resolved = resolve_web_dist().expect("resolve");
        assert_eq!(resolved, dir.path());
    }

    #[test]
    fn invalid_env_override_errors() {
        let _restore = EnvRestore::new();
        unsafe { std::env::set_var("LITECODE_WEB_DIST", "/nonexistent/web/dist") };

        let err = resolve_web_dist().expect_err("should fail");
        assert!(err.to_string().contains("LITECODE_WEB_DIST"));
    }
}
