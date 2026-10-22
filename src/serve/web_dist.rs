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
    
    



}
