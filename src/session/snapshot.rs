use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use git2::{Repository, RepositoryInitOptions};

use crate::config::git_install::find_git_exe;
use crate::session::snapshot_paths::assert_snapshots_isolated;
use crate::types::{LitecodeError, Result};
use crate::workspace::filter::snapshot_exclude_dir_basenames;

const SNAPSHOT_EXCLUDE_MARKER: &str = "# litecode-snapshot-excludes";

/// Binary / media / IDE / Unity-generated blobs — hashing these dominates game trees.
/// Keep agent-relevant text: `.cs`, shaders, `.prefab`/`.unity`/`.asset`, configs, docs.
const SNAPSHOT_SKIP_EXTENSIONS: &[&str] = &[
    // images / video / audio / 3D
    "png",
    "jpg",
    "jpeg",
    "gif",
    "bmp",
    "tga",
    "psd",
    "tif",
    "tiff",
    "webp",
    "exr",
    "hdr",
    "fbx",
    "obj",
    "blend",
    "dae",
    "gltf",
    "glb",
    "3ds",
    "wav",
    "mp3",
    "ogg",
    "aiff",
    "flac",
    "mp4",
    "mov",
    "avi",
    "webm",
    // native / archives
    "dll",
    "so",
    "dylib",
    "a",
    "lib",
    "pdb",
    "exe",
    "unitypackage",
    "zip",
    "7z",
    "rar",
    "gz",
    "tar",
    "bin",
    "bytes",
    // Unity sidecar / generated (Agents edit .cs/.prefab/.unity, not these)
    "meta",
    "anim",
    "mat",
    "controller",
    "overridecontroller",
    "mask",
    "vfx",
    "fontsettings",
    "ttf",
    "otf",
    "fnt",
    "ttf~",
    "png~",
    "terrainlayer",
    "rendertexture",
    "cubemap",
    "guiskin",
    "physicmaterial",
    "mixer",
    "signal",
    "spriteatlas",
    "spriteatlasv2",
    // IDE / VS glue
    "csproj",
    "sln",
    "userprefs",
    "suo",
    "pidb",
    "mdb",
    "opendb",
    "VC.db",
    "lscache",
];

/// Skip individual files larger than this (bytes).
const SNAPSHOT_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Per-workspace mutexes to serialise index writes in `snapshot_track`.
static SNAPSHOT_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Age after which a left-over `index.lock` is treated as stale (crashed/killed process).
const STALE_INDEX_LOCK_SECS: u64 = 600;

/// Get (or create) the per-workspace mutex for the LAP workspace path.
fn workspace_snapshot_lock(workspace: &Path) -> Arc<Mutex<()>> {
    let key = crate::config::path::canon_abs_lossy(workspace);
    let mut map = SNAPSHOT_LOCKS.lock().unwrap();
    map.entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Per-session snap cap (FIFO eviction).
pub const MAX_ANCHORS: usize = 100;
/// Drop snapshots for sessions not touched in this many days.
pub const SNAPSHOT_RETENTION_DAYS: u64 = 30;

const SNAPSHOT_REF_PREFIX: &str = "refs/snapshots/";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotMaintenanceReport {
    pub orphans_removed: usize,
    pub stale_removed: usize,
}

// ── repo helpers ──

/// Open or initialize a **bare** snapshot git repo at `snapshots_dir`, workdir = workspace.
///
/// Layout mirrors OpenCode: external git dir + worktree = project (`GIT_DIR` / `GIT_WORK_TREE`).
/// When the workspace is a git repo, seeds read-only `alternates` + copies the project index
/// so unchanged blobs are not re-hashed (OpenCode-style). Never writes into project `.git`.
fn open_or_init_repo(workspace: &Path, snapshots_dir: &Path) -> Result<Repository> {
    if let Err(msg) = assert_snapshots_isolated(workspace, snapshots_dir) {
        return Err(LitecodeError::Config(msg));
    }

    let mut freshly_inited = false;
    let repo = match Repository::open_bare(snapshots_dir) {
        Ok(r) => r,
        Err(_) => match Repository::open(snapshots_dir) {
            Ok(r) => r,
            Err(_) => {
                fs::create_dir_all(snapshots_dir)?;
                let mut opts = RepositoryInitOptions::new();
                opts.bare(true);
                freshly_inited = true;
                Repository::init_opts(snapshots_dir, &opts)?
            }
        },
    };

    repo.set_workdir(workspace, false)?;
    // OpenCode sets these on shadow-git init (Windows path / CRLF correctness).
    configure_snapshot_repo(&repo, snapshots_dir, freshly_inited);
    if freshly_inited {
        seed_from_project_git(workspace, snapshots_dir);
    }
    sanitize_shadow_alternates(workspace, snapshots_dir);

    // Re-open so libgit2 loads any alternates written during seed (odb is fixed at open).
    let repo = Repository::open_bare(snapshots_dir).or_else(|_| Repository::open(snapshots_dir))?;
    repo.set_workdir(workspace, false)?;
    Ok(repo)
}

/// Shadow-git knobs aligned with OpenCode `snapshot/index.ts` init.
fn configure_snapshot_repo(repo: &Repository, snapshots_dir: &Path, freshly_inited: bool) {
    let Ok(mut cfg) = repo.config() else {
        write_snapshot_info_exclude(repo);
        return;
    };
    let _ = cfg.set_bool("core.autocrlf", false);
    let _ = cfg.set_bool("core.longpaths", true);
    let _ = cfg.set_bool("core.symlinks", true);
    let _ = cfg.set_bool("core.fsmonitor", false);
    if freshly_inited {
        let _ = cfg.set_bool("core.untrackedCache", true);
        // Large-worktree knobs via CLI (not all exposed uniformly via git2).
        let _ = git_config_set(snapshots_dir, "feature.manyFiles", "true");
        let _ = git_config_set(snapshots_dir, "index.version", "4");
        let _ = git_config_set(snapshots_dir, "index.threads", "true");
        let _ = git_config_set(snapshots_dir, "core.untrackedCache", "true");
    }
    write_snapshot_info_exclude(repo);
}

/// Write `$GIT_DIR/info/exclude` so git/libgit2 can prune heavy trees during add.
fn write_snapshot_info_exclude(repo: &Repository) {
    let exclude = repo.path().join("info").join("exclude");
    if let Some(parent) = exclude.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut body = String::from(SNAPSHOT_EXCLUDE_MARKER);
    body.push('\n');
    for name in snapshot_exclude_dir_basenames() {
        body.push_str(name);
        body.push_str("/\n");
    }
    let _ = fs::write(exclude, body);
}

/// True when `rel` should never enter the snapshot index (path-only checks).
fn snapshot_path_excluded(rel: &str) -> bool {
    let n = normalize_rel(rel);
    if n.is_empty() {
        return true;
    }
    // Directory / submodule coalesced entries as `foo/` cannot live in the index.
    if n.ends_with('/') {
        return true;
    }
    for seg in n.split('/') {
        if snapshot_exclude_dir_basenames().iter().any(|d| d == seg) {
            return true;
        }
    }
    if let Some(ext) = Path::new(&n).extension().and_then(|e| e.to_str()) {
        let lower = ext.to_ascii_lowercase();
        if SNAPSHOT_SKIP_EXTENSIONS.iter().any(|e| *e == lower) {
            return true;
        }
    }
    false
}

fn snapshot_workdir_file_excluded(workdir: &Path, rel: &str) -> bool {
    if snapshot_path_excluded(rel) {
        return true;
    }
    let full = workdir.join(rel);
    match fs::metadata(&full) {
        Ok(meta) if meta.is_file() && meta.len() > SNAPSHOT_MAX_FILE_BYTES => true,
        _ => false,
    }
}

/// Remove `index.lock` when it is older than [`STALE_INDEX_LOCK_SECS`].
fn clear_stale_index_lock(git_dir: &Path) {
    let lock = git_dir.join("index.lock");
    if !lock.is_file() {
        return;
    }
    let stale = fs::metadata(&lock)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > Duration::from_secs(STALE_INDEX_LOCK_SECS));
    if !stale {
        return;
    }
    match fs::remove_file(&lock) {
        Ok(()) => tracing::warn!(
            path = %lock.display(),
            "removed stale snapshot index.lock (process likely crashed mid-write)"
        ),
        Err(e) => tracing::warn!(
            path = %lock.display(),
            error = %e,
            "failed to remove stale snapshot index.lock"
        ),
    }
}

fn force_clear_index_lock(git_dir: &Path) {
    let lock = git_dir.join("index.lock");
    if lock.is_file() {
        let _ = fs::remove_file(&lock);
    }
}

fn require_git_exe() -> Result<PathBuf> {
    find_git_exe().ok_or_else(|| {
        LitecodeError::Config(
            "git executable not found (required for workspace snapshot track/patch)".into(),
        )
    })
}

fn git_cmd(git: &Path, git_dir: &Path, worktree: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(git);
    cmd.arg("-c")
        .arg("core.autocrlf=false")
        .arg("-c")
        .arg("core.longpaths=true")
        .arg("-c")
        .arg("core.symlinks=true")
        .arg("-c")
        .arg("core.quotepath=false")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(worktree)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn git_run(
    git: &Path,
    git_dir: &Path,
    worktree: &Path,
    args: &[&str],
) -> Result<(i32, String, String)> {
    let output = git_cmd(git, git_dir, worktree, args)
        .output()
        .map_err(|e| {
            LitecodeError::Config(format!("failed to spawn git {}: {e}", args.join(" ")))
        })?;
    let code = output.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((code, stdout, stderr))
}

fn git_run_stdin(
    git: &Path,
    git_dir: &Path,
    worktree: &Path,
    args: &[&str],
    stdin: &[u8],
) -> Result<(i32, String, String)> {
    let mut cmd = git_cmd(git, git_dir, worktree, args);
    cmd.stdin(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        LitecodeError::Config(format!("failed to spawn git {}: {e}", args.join(" ")))
    })?;
    if let Some(mut input) = child.stdin.take() {
        use std::io::Write;
        let _ = input.write_all(stdin);
    }
    let output = child
        .wait_with_output()
        .map_err(|e| LitecodeError::Config(format!("git {} wait failed: {e}", args.join(" "))))?;
    let code = output.status.code().unwrap_or(1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((code, stdout, stderr))
}

fn git_config_set(git_dir: &Path, key: &str, value: &str) -> Result<()> {
    let git = match find_git_exe() {
        Some(g) => g,
        None => return Ok(()),
    };
    let mut cmd = Command::new(&git);
    let output = cmd
        .arg("--git-dir")
        .arg(git_dir)
        .args(["config", key, value])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| LitecodeError::Config(format!("git config {key}: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::debug!(key, value, %stderr, "git config set soft-failed");
    }
    Ok(())
}

/// OpenCode-style seed: read-only alternates into project objects + copy project index.
/// Best-effort; never writes into the project `.git`.
fn seed_from_project_git(workspace: &Path, snapshots_dir: &Path) {
    let Ok(git) = require_git_exe() else {
        return;
    };
    let discover = Command::new(&git)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = discover else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let source = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if source.is_empty() {
        return;
    }
    let source_git = PathBuf::from(&source);
    if !source_git.exists() {
        return;
    }
    // Refuse to seed from a git dir inside the snapshot dir (shouldn't happen).
    if crate::config::path::is_under(&source_git, snapshots_dir) {
        return;
    }

    let source_objects = source_git.join("objects");
    let mut alternates: Vec<PathBuf> = Vec::new();
    if source_objects.is_dir() {
        alternates.push(source_objects.clone());
    }
    let chained = source_objects.join("info").join("alternates");
    if let Ok(text) = fs::read_to_string(&chained) {
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let candidate = PathBuf::from(line);
            if candidate.is_dir() {
                alternates.push(candidate);
            }
        }
    }
    if alternates.is_empty() {
        return;
    }

    if let Err(e) = write_shadow_alternates(snapshots_dir, &alternates) {
        tracing::warn!(error = %e, "snapshot seed: failed to write alternates");
        return;
    }

    let source_index = source_git.join("index");
    let dest_index = snapshots_dir.join("index");
    if source_index.is_file() {
        if let Err(e) = fs::copy(&source_index, &dest_index) {
            tracing::debug!(error = %e, "snapshot seed: index copy skipped");
        } else {
            tracing::info!(
                source = %source_git.display(),
                "snapshot seeded from project git (alternates + index)"
            );
        }
    } else {
        tracing::info!(
            source = %source_git.display(),
            "snapshot seeded from project git (alternates only)"
        );
    }
}

fn write_shadow_alternates(snapshots_dir: &Path, lines: &[PathBuf]) -> Result<()> {
    let info = snapshots_dir.join("objects").join("info");
    fs::create_dir_all(&info)?;
    let path = info.join("alternates");
    let mut body = String::new();
    for p in lines {
        // Prefer absolute paths so the shadow repo resolves blobs regardless of cwd.
        let abs = crate::config::path::canon_abs_lossy(p);
        body.push_str(&abs.display().to_string());
        body.push('\n');
    }
    fs::write(&path, body)?;
    Ok(())
}

/// Drop alternates that are missing or that point inside the workspace *outside*
/// project `.git` (unsafe). Project `objects` under `<workspace>/.git` are allowed
/// — that is the OpenCode seed path.
fn sanitize_shadow_alternates(workspace: &Path, snapshots_dir: &Path) {
    let path = snapshots_dir
        .join("objects")
        .join("info")
        .join("alternates");
    if !path.is_file() {
        return;
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let project_git = workspace.join(".git");
    let mut kept: Vec<PathBuf> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let p = PathBuf::from(line);
        if !p.exists() {
            continue;
        }
        let under_ws = crate::config::path::is_under(&p, workspace);
        let under_project_git = crate::config::path::is_under(&p, &project_git);
        if under_ws && !under_project_git {
            tracing::warn!(
                path = %p.display(),
                "dropping snapshot alternate under workspace (outside .git)"
            );
            continue;
        }
        kept.push(p);
    }
    if kept.is_empty() {
        let _ = fs::remove_file(&path);
        return;
    }
    let _ = write_shadow_alternates(snapshots_dir, &kept);
}

fn encode_nul_pathspecs(files: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in files {
        out.extend_from_slice(format!(":(top,literal){f}").as_bytes());
        out.push(0);
    }
    out
}

fn split_nul_paths(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|s| !s.is_empty())
        .map(|s| normalize_rel(s))
        .filter(|s| !s.is_empty() && !s.ends_with('/'))
        .collect()
}

fn path_is_dir(workdir: &Path, rel: &str) -> bool {
    fs::metadata(workdir.join(rel))
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

/// Paths that current exclude rules would ignore, including already-tracked ones
/// (`--no-index`) so a later `.gitignore` change unstages them from the snapshot.
fn git_check_ignore_set(
    git: &Path,
    git_dir: &Path,
    worktree: &Path,
    rels: &[String],
) -> Result<HashSet<String>> {
    if rels.is_empty() {
        return Ok(HashSet::new());
    }
    let mut stdin = Vec::new();
    for rel in rels {
        stdin.extend_from_slice(rel.as_bytes());
        stdin.push(0);
    }
    let (code, stdout, stderr) = git_run_stdin(
        git,
        git_dir,
        worktree,
        &["check-ignore", "--no-index", "-z", "--stdin"],
        &stdin,
    )?;
    // 0 = some matches, 1 = none; anything else is a hard failure.
    if code != 0 && code != 1 {
        return Err(LitecodeError::Config(format!(
            "snapshot check-ignore failed (code {code}): {stderr}"
        )));
    }
    Ok(split_nul_paths(&stdout).into_iter().collect())
}

/// OpenCode-style incremental stage via native git, then `write-tree`.
fn snapshot_write_tree(workspace: &Path, git_dir: &Path) -> Result<git2::Oid> {
    clear_stale_index_lock(git_dir);
    for attempt in 0..2 {
        match cli_sync_and_write_tree(workspace, git_dir) {
            Ok(oid) => return Ok(oid),
            Err(e) if attempt == 0 && is_index_lock_message(&e) => {
                force_clear_index_lock(git_dir);
                tracing::warn!(
                    path = %git_dir.join("index.lock").display(),
                    "removed snapshot index.lock after Locked error"
                );
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("snapshot_write_tree retry loop")
}

fn is_index_lock_message(err: &LitecodeError) -> bool {
    let s = err.to_string().to_ascii_lowercase();
    s.contains("index.lock")
        || (s.contains("unable to create") && s.contains("lock"))
        || s.contains("index is locked")
}

fn cli_sync_and_write_tree(workspace: &Path, git_dir: &Path) -> Result<git2::Oid> {
    let git = require_git_exe()?;

    // Ensure exclude file exists (also written on init; refresh for long-lived repos).
    if let Ok(repo) = Repository::open_bare(git_dir).or_else(|_| Repository::open(git_dir)) {
        write_snapshot_info_exclude(&repo);
    }

    let (diff_code, diff_out, diff_err) = git_run(
        &git,
        git_dir,
        workspace,
        &["diff-files", "--name-only", "-z", "--", "."],
    )?;
    let (other_code, other_out, other_err) = git_run(
        &git,
        git_dir,
        workspace,
        &[
            "ls-files",
            "--full-name",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
    )?;
    if diff_code != 0 || other_code != 0 {
        return Err(LitecodeError::Config(format!(
            "snapshot list failed: diff={diff_code} ({diff_err}) others={other_code} ({other_err})"
        )));
    }

    let mut all: Vec<String> = split_nul_paths(&diff_out);
    for p in split_nul_paths(&other_out) {
        if !all.iter().any(|x| x == &p) {
            all.push(p);
        }
    }

    // `--exclude-standard` only hides *untracked* ignored files. Paths already
    // in the shadow index, directory pathspecs, or files ignored after a
    // .gitignore change still show up here; `git add` then exits 1 and aborts
    // the whole tree. Drop those from the index instead of adding them.
    let ignored = git_check_ignore_set(&git, git_dir, workspace, &all)?;

    let mut allow: Vec<String> = Vec::new();
    let mut drop_cached: Vec<String> = Vec::new();
    for rel in all {
        if snapshot_workdir_file_excluded(workspace, &rel) || ignored.contains(&rel) {
            drop_cached.push(rel);
            continue;
        }
        // Directory pathspecs make `git add` recurse and die on ignored
        // children. Files inside a real directory are listed separately.
        if path_is_dir(workspace, &rel) {
            continue;
        }
        allow.push(rel);
    }

    if !drop_cached.is_empty() {
        let stdin = encode_nul_pathspecs(&drop_cached);
        let _ = git_run_stdin(
            &git,
            git_dir,
            workspace,
            &[
                "rm",
                "--cached",
                "-r",
                "-f",
                "--ignore-unmatch",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
            &stdin,
        )?;
    }

    if !allow.is_empty() {
        let stdin = encode_nul_pathspecs(&allow);
        let (code, _, stderr) = git_run_stdin(
            &git,
            git_dir,
            workspace,
            &[
                "add",
                "--all",
                "--sparse",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
            &stdin,
        )?;
        if code != 0 {
            return Err(LitecodeError::Config(format!(
                "snapshot git add failed (code {code}): {stderr}"
            )));
        }
    }

    let (code, stdout, stderr) = git_run(&git, git_dir, workspace, &["write-tree"])?;
    if code != 0 {
        return Err(LitecodeError::Config(format!(
            "snapshot write-tree failed (code {code}): {stderr}"
        )));
    }
    let hash = stdout.trim();
    git2::Oid::from_str(hash)
        .map_err(|e| LitecodeError::Config(format!("invalid write-tree oid '{hash}': {e}")))
}

/// Build a ref name for a session + anchor k.
fn snapshot_ref(session_id: &str, k: i64) -> String {
    format!("{SNAPSHOT_REF_PREFIX}{session_id}/{k}")
}

/// Return all snapshot anchor refs for a session.
fn session_refs(repo: &Repository, session_id: &str) -> Result<Vec<(i64, git2::Oid)>> {
    let prefix = format!("{SNAPSHOT_REF_PREFIX}{session_id}/");
    let mut refs = Vec::new();
    for entry in repo.references()?.flatten() {
        let name = entry.name().unwrap_or("");
        if let Some(k_str) = name.strip_prefix(&prefix) {
            if let Ok(k) = k_str.parse::<i64>() {
                if let Some(oid) = entry.target() {
                    refs.push((k, oid));
                }
            }
        }
    }
    Ok(refs)
}

/// Return all session IDs that have snapshot refs.
fn all_session_ids(repo: &Repository) -> Result<HashSet<String>> {
    let mut ids = HashSet::new();
    let prefix = SNAPSHOT_REF_PREFIX;
    for entry in repo.references()?.flatten() {
        let name = entry.name().unwrap_or("");
        if let Some(rest) = name.strip_prefix(prefix) {
            if let Some((session_id, _)) = rest.split_once('/') {
                ids.insert(session_id.to_string());
            }
        }
    }
    Ok(ids)
}

/// Delete all refs for a session.
fn delete_session_refs(repo: &Repository, session_id: &str) -> Result<()> {
    let prefix = format!("{SNAPSHOT_REF_PREFIX}{session_id}/");
    let names: Vec<String> = repo
        .references()?
        .flatten()
        .filter_map(|r| {
            let name = r.name()?.to_string();
            name.starts_with(&prefix).then_some(name)
        })
        .collect();
    for name in &names {
        if let Ok(mut r) = repo.find_reference(name) {
            let _ = r.delete();
        }
    }
    Ok(())
}

/// Top-level names that must never be touched by file-level restore.
fn is_protected_rel(rel: &str) -> bool {
    let rel = rel.trim_start_matches("./");
    rel == ".git" || rel == ".litecode" || rel.starts_with(".git/") || rel.starts_with(".litecode/")
}

fn normalize_rel(path: &str) -> String {
    path.trim_start_matches("./").replace('\\', "/")
}

fn patches_dir(snapshots_dir: &Path, session_id: &str) -> PathBuf {
    snapshots_dir.join("patches").join(session_id)
}

fn patch_path(snapshots_dir: &Path, session_id: &str, k: i64) -> PathBuf {
    patches_dir(snapshots_dir, session_id).join(format!("{k}.json"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum PatchStatus {
    #[default]
    Ok,
    TrackFailed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct PatchFile {
    files: Vec<String>,
    /// Absent in legacy patches → treated as [`PatchStatus::Ok`].
    #[serde(default)]
    status: PatchStatus,
}

/// Outcome of file-level restore. Callers must not treat non-`Restored` as success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    Restored { files: Vec<String> },
    NothingToRevert,
    Unavailable { reason: RestoreUnavailable },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreUnavailable {
    MissingTrackRef,
    TrackFailed,
}

/// Result of recording a turn patch (may mark track failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordPatchResult {
    pub files: Vec<String>,
    pub track_failed: bool,
}

fn write_patch(
    snapshots_dir: &Path,
    session_id: &str,
    k: i64,
    files: &[String],
    status: PatchStatus,
) -> Result<()> {
    let path = patch_path(snapshots_dir, session_id, k);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = PatchFile {
        files: files.to_vec(),
        status,
    };
    fs::write(&path, serde_json::to_vec_pretty(&body)?)?;
    Ok(())
}

fn read_patch_file(snapshots_dir: &Path, session_id: &str, k: i64) -> Option<PatchFile> {
    let path = patch_path(snapshots_dir, session_id, k);
    let bytes = fs::read(&path).ok()?;
    serde_json::from_slice::<PatchFile>(&bytes).ok()
}

fn read_patch(snapshots_dir: &Path, session_id: &str, k: i64) -> Vec<String> {
    read_patch_file(snapshots_dir, session_id, k)
        .map(|p| p.files)
        .unwrap_or_default()
}

/// True if any patch at anchors `>= k` was recorded as track failure.
fn any_track_failed_from(snapshots_dir: &Path, session_id: &str, k: i64) -> bool {
    let dir = patches_dir(snapshots_dir, session_id);
    let Ok(entries) = fs::read_dir(&dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|s| s.strip_suffix(".json")) else {
            continue;
        };
        let Ok(ak) = stem.parse::<i64>() else {
            continue;
        };
        if ak < k {
            continue;
        }
        if read_patch_file(snapshots_dir, session_id, ak)
            .is_some_and(|p| p.status == PatchStatus::TrackFailed)
        {
            return true;
        }
    }
    false
}

fn delete_patch(snapshots_dir: &Path, session_id: &str, k: i64) {
    let path = patch_path(snapshots_dir, session_id, k);
    let _ = fs::remove_file(path);
}

fn delete_session_patches(snapshots_dir: &Path, session_id: &str) {
    let dir = patches_dir(snapshots_dir, session_id);
    let _ = fs::remove_dir_all(dir);
}

fn patch_has_revertible_files(patch: &PatchFile) -> bool {
    if patch.status == PatchStatus::TrackFailed {
        return false;
    }
    patch.files.iter().any(|f| {
        let n = normalize_rel(f);
        !n.is_empty() && !is_protected_rel(&n)
    })
}

/// Highest user-anchor `k` whose recorded patch lists at least one revertible file.
/// `None` means no user message has a file-level revert available.
pub fn max_file_revert_k(snapshots_dir: &Path, session_id: &str) -> Option<i64> {
    let dir = patches_dir(snapshots_dir, session_id);
    let Ok(entries) = fs::read_dir(&dir) else {
        return None;
    };
    let mut max_k: Option<i64> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|s| s.strip_suffix(".json")) else {
            continue;
        };
        let Ok(ak) = stem.parse::<i64>() else {
            continue;
        };
        let Some(patch) = read_patch_file(snapshots_dir, session_id, ak) else {
            continue;
        };
        if patch_has_revertible_files(&patch) {
            max_k = Some(max_k.map_or(ak, |m| m.max(ak)));
        }
    }
    max_k
}

/// Union of patch file lists for all anchors `>= k` (OpenCode multi-step undo).
fn union_patches_from(snapshots_dir: &Path, session_id: &str, k: i64) -> Vec<String> {
    let dir = patches_dir(snapshots_dir, session_id);
    let mut set = HashSet::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.to_str().and_then(|s| s.strip_suffix(".json")) else {
            continue;
        };
        let Ok(ak) = stem.parse::<i64>() else {
            continue;
        };
        if ak < k {
            continue;
        }
        for f in read_patch(snapshots_dir, session_id, ak) {
            let n = normalize_rel(&f);
            if !n.is_empty() && !is_protected_rel(&n) {
                set.insert(n);
            }
        }
    }
    let mut files: Vec<_> = set.into_iter().collect();
    files.sort();
    files
}

fn diff_tree_paths(repo: &Repository, before: git2::Oid, after: git2::Oid) -> Result<Vec<String>> {
    let old_tree = repo.find_tree(before)?;
    let new_tree = repo.find_tree(after)?;
    let diff = repo.diff_tree_to_tree(Some(&old_tree), Some(&new_tree), None)?;
    let mut files = HashSet::new();
    diff.foreach(
        &mut |delta, _| {
            if let Some(p) = delta.new_file().path().or_else(|| delta.old_file().path()) {
                let n = normalize_rel(&p.to_string_lossy());
                if !n.is_empty() && !is_protected_rel(&n) {
                    files.insert(n);
                }
            }
            true
        },
        None,
        None,
        None,
    )?;
    let mut out: Vec<_> = files.into_iter().collect();
    out.sort();
    Ok(out)
}

fn tree_has_path(tree: &git2::Tree, rel: &str) -> bool {
    tree.get_path(Path::new(rel)).is_ok()
}

fn checkout_path(repo: &Repository, tree: &git2::Tree, rel: &str) -> Result<()> {
    let mut opts = git2::build::CheckoutBuilder::new();
    opts.force().path(rel);
    repo.checkout_tree(tree.as_object(), Some(&mut opts))?;
    Ok(())
}

fn remove_workdir_path(workspace: &Path, rel: &str) -> Result<()> {
    let full = workspace.join(rel);
    if full.is_dir() {
        fs::remove_dir_all(&full)?;
    } else if full.exists() {
        fs::remove_file(&full)?;
    }
    Ok(())
}

// ── public API ──

/// Initialize the snapshot git repo for a workspace. Idempotent.
pub fn init_snapshot_repo(workspace: &Path, snapshots_dir: &Path) -> Result<()> {
    open_or_init_repo(workspace, snapshots_dir)?;
    Ok(())
}

/// Warm the shadow index on workspace open so the first turn can use incremental track.
///
/// With project git: seed (on init) + one CLI incremental sync.
/// Without: CLI sync bootstraps from untracked files. Failures are logged by callers.
pub fn warm_snapshot_repo(workspace: &Path, snapshots_dir: &Path) -> Result<()> {
    let snapshot_lock = workspace_snapshot_lock(workspace);
    let _lock = snapshot_lock.lock().expect("snapshot lock poisoned");

    let repo = open_or_init_repo(workspace, snapshots_dir)?;
    clear_stale_index_lock(repo.path());
    // Always run one incremental sync so an empty (unseeded) index picks up workdir files.
    let _ = snapshot_write_tree(workspace, repo.path())?;
    tracing::info!(
        path = %snapshots_dir.display(),
        "snapshot index warmed"
    );
    Ok(())
}

/// Take a snapshot of the workspace at anchor `k`.
/// Call at turn start before any tools execute.
/// Records a git tree ref `refs/snapshots/{session_id}/{k}`.
pub fn snapshot_track(
    workspace: &Path,
    snapshots_dir: &Path,
    session_id: &str,
    k: i64,
) -> Result<()> {
    let snapshot_lock = workspace_snapshot_lock(workspace);
    let _lock = snapshot_lock.lock().expect("snapshot lock poisoned");

    let repo = open_or_init_repo(workspace, snapshots_dir)?;

    // FIFO eviction: keep at most MAX_ANCHORS per session.
    let mut existing = session_refs(&repo, session_id)?;
    existing.sort_by_key(|(k, _)| *k);
    if !existing.iter().any(|(ek, _)| *ek == k) && existing.len() >= MAX_ANCHORS {
        if let Some((oldest_k, _)) = existing.first() {
            if let Ok(mut r) = repo.find_reference(&snapshot_ref(session_id, *oldest_k)) {
                let _ = r.delete();
            }
            delete_patch(snapshots_dir, session_id, *oldest_k);
        }
    }

    let tree_oid = snapshot_write_tree(workspace, repo.path())?;
    // CLI write-tree may add objects git2 has not yet seen in this handle.
    drop(repo);
    let repo = Repository::open_bare(snapshots_dir).or_else(|_| Repository::open(snapshots_dir))?;
    repo.set_workdir(workspace, false)?;
    repo.reference(
        &snapshot_ref(session_id, k),
        tree_oid,
        true,
        "litecode snapshot track",
    )?;

    tracing::debug!(session_id, k, tree = %tree_oid, "snapshot tracked");
    Ok(())
}

/// OpenCode-style: after tools run, record which paths changed since `tree_k`.
///
/// Writes `patches/{session_id}/{k}.json`. Call at turn end.
///
/// If the turn-start track ref is missing, writes `status: track_failed` so restore
/// can distinguish "no edits" from "snapshot unavailable".
pub fn snapshot_record_patch(
    workspace: &Path,
    snapshots_dir: &Path,
    session_id: &str,
    k: i64,
) -> Result<RecordPatchResult> {
    let snapshot_lock = workspace_snapshot_lock(workspace);
    let _lock = snapshot_lock.lock().expect("snapshot lock poisoned");

    let repo = open_or_init_repo(workspace, snapshots_dir)?;
    let before = match repo.refname_to_id(&snapshot_ref(session_id, k)) {
        Ok(oid) => oid,
        Err(_) => {
            tracing::warn!(session_id, k, "snapshot_record_patch: missing track ref");
            write_patch(snapshots_dir, session_id, k, &[], PatchStatus::TrackFailed)?;
            return Ok(RecordPatchResult {
                files: Vec::new(),
                track_failed: true,
            });
        }
    };

    let after = snapshot_write_tree(workspace, repo.path())?;
    drop(repo);
    let repo = Repository::open_bare(snapshots_dir).or_else(|_| Repository::open(snapshots_dir))?;
    repo.set_workdir(workspace, false)?;

    let files = diff_tree_paths(&repo, before, after)?;
    write_patch(snapshots_dir, session_id, k, &files, PatchStatus::Ok)?;
    tracing::debug!(
        session_id,
        k,
        count = files.len(),
        "snapshot patch recorded"
    );
    Ok(RecordPatchResult {
        files,
        track_failed: false,
    })
}

/// Restore **only** files touched by turns at anchors `>= k` (OpenCode file-level undo).
///
/// Never does a whole-tree checkout. Paths outside the patch union are left alone.
///
/// Returns [`RestoreOutcome`]: callers must treat non-`Restored` as unsuccessful for UX.
pub fn snapshot_restore(
    workspace: &Path,
    snapshots_dir: &Path,
    session_id: &str,
    k: i64,
    user_detail_count: i64,
) -> Result<RestoreOutcome> {
    if k < 0 || k >= user_detail_count {
        return Err(LitecodeError::InvalidRevertAnchor(format!(
            "invalid revert anchor k={k} (user items={user_detail_count})"
        )));
    }

    let snapshot_lock = workspace_snapshot_lock(workspace);
    let _lock = snapshot_lock.lock().expect("snapshot lock poisoned");

    let repo = open_or_init_repo(workspace, snapshots_dir)?;
    let tree_oid = match repo.refname_to_id(&snapshot_ref(session_id, k)) {
        Ok(oid) => oid,
        Err(_) => {
            tracing::warn!(session_id, k, "snapshot_restore: missing track ref");
            return Ok(RestoreOutcome::Unavailable {
                reason: RestoreUnavailable::MissingTrackRef,
            });
        }
    };

    if any_track_failed_from(snapshots_dir, session_id, k) {
        tracing::warn!(
            session_id,
            k,
            "snapshot_restore: track_failed patch present"
        );
        return Ok(RestoreOutcome::Unavailable {
            reason: RestoreUnavailable::TrackFailed,
        });
    }

    let tree = repo.find_tree(tree_oid)?;
    let files = union_patches_from(snapshots_dir, session_id, k);
    if files.is_empty() {
        tracing::info!(
            session_id,
            k,
            "snapshot_restore: empty patch union (nothing to revert)"
        );
        return Ok(RestoreOutcome::NothingToRevert);
    }

    // Phase 1: collect the full operation set BEFORE mutating anything, so a
    // bad path fails before any file is touched (non-atomic restore fix).
    let mut to_checkout: Vec<&str> = Vec::new();
    let mut to_remove: Vec<&str> = Vec::new();
    for rel in &files {
        if is_protected_rel(rel) {
            continue;
        }
        // Sanitize: patch paths must be workspace-relative with no traversal.
        // Absolute paths or `..` components could escape the workspace (F).
        let rel_clean = rel.trim_start_matches("./");
        if std::path::Path::new(rel_clean).is_absolute()
            || rel_clean.split(['/', '\\']).any(|c| c == "..")
        {
            tracing::warn!(rel, "snapshot_restore: rejecting unsafe patch path");
            continue;
        }
        if tree_has_path(&tree, rel) {
            to_checkout.push(rel);
        } else {
            to_remove.push(rel);
        }
    }
    if to_checkout.is_empty() && to_remove.is_empty() {
        return Ok(RestoreOutcome::NothingToRevert);
    }

    // Phase 2a: apply all checkouts in ONE git operation (all-or-error at the
    // git level), then removals.
    if !to_checkout.is_empty() {
        let mut opts = git2::build::CheckoutBuilder::new();
        opts.force();
        for rel in &to_checkout {
            opts.path(rel);
        }
        repo.checkout_tree(tree.as_object(), Some(&mut opts))?;
    }

    // Phase 2b: removals; on failure roll back the removals already applied by
    // re-checking them out from the restore tree.
    let mut restored: Vec<String> = to_checkout.iter().map(|s| s.to_string()).collect();
    for rel in to_remove {
        if let Err(e) = remove_workdir_path(workspace, rel) {
            for applied in &restored {
                let _ = checkout_path(&repo, &tree, applied);
            }
            return Err(e);
        }
        restored.push(rel.to_string());
    }

    tracing::info!(
        session_id,
        k,
        files = restored.len(),
        "snapshot restored (file-level)"
    );
    Ok(RestoreOutcome::Restored { files: restored })
}

// ── exists check ──

/// Check if a snapshot ref exists for the given session + anchor.
pub fn snapshot_exists(snapshots_dir: &Path, session_id: &str, k: i64) -> bool {
    let Ok(repo) =
        Repository::open_bare(snapshots_dir).or_else(|_| Repository::open(snapshots_dir))
    else {
        return false;
    };
    repo.find_reference(&snapshot_ref(session_id, k)).is_ok()
}

// ── maintenance ──

/// Remove all snapshot refs and patches for a session.
pub fn delete_session_snapshots(snapshots_dir: &Path, session_id: &str) -> Result<()> {
    let Ok(repo) =
        Repository::open_bare(snapshots_dir).or_else(|_| Repository::open(snapshots_dir))
    else {
        delete_session_patches(snapshots_dir, session_id);
        return Ok(());
    };
    delete_session_refs(&repo, session_id)?;
    delete_session_patches(snapshots_dir, session_id);
    Ok(())
}

/// Remove orphan (dead session) and stale (untouched N days) snapshot refs.
pub fn maintain_snapshots(
    snapshots_dir: &Path,
    sessions_db: &Path,
) -> Result<SnapshotMaintenanceReport> {
    let Ok(repo) =
        Repository::open_bare(snapshots_dir).or_else(|_| Repository::open(snapshots_dir))
    else {
        return Ok(SnapshotMaintenanceReport::default());
    };
    if !sessions_db.is_file() {
        return Ok(SnapshotMaintenanceReport::default());
    }

    let active_ids = session_ids_from_db(sessions_db).unwrap_or_default();
    let active: HashSet<_> = active_ids.into_iter().collect();
    let snapshot_sessions = all_session_ids(&repo).unwrap_or_default();
    let cutoff = snapshot_cutoff_time(SNAPSHOT_RETENTION_DAYS);

    let mut report = SnapshotMaintenanceReport::default();

    for sid in &snapshot_sessions {
        if !active.contains(sid.as_str()) {
            delete_session_refs(&repo, sid)?;
            delete_session_patches(snapshots_dir, sid);
            report.orphans_removed += 1;
            continue;
        }

        let stale = session_refs(&repo, sid)
            .map(|refs| {
                if refs.is_empty() {
                    return true;
                }
                repo_mtime_before(&repo, cutoff).unwrap_or(false)
            })
            .unwrap_or(false);

        if stale {
            delete_session_refs(&repo, sid)?;
            delete_session_patches(snapshots_dir, sid);
            report.stale_removed += 1;
        }
    }

    Ok(report)
}

fn repo_mtime_before(repo: &Repository, cutoff: SystemTime) -> Result<bool> {
    let meta = fs::metadata(repo.path())?;
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    Ok(modified < cutoff)
}

// ── helpers ──

fn session_ids_from_db(sessions_db: &Path) -> Result<Vec<String>> {
    let conn = rusqlite::Connection::open(sessions_db)?;
    let mut stmt = conn.prepare("SELECT id FROM sessions")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn snapshot_cutoff_time(retention_days: u64) -> SystemTime {
    UNIX_EPOCH
        + Duration::from_secs(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                .saturating_sub(retention_days * 24 * 60 * 60),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::snapshot_paths::{
        SNAPSHOTS_ROOT_ENV, path_is_under, snapshots_dir_for_workspace,
    };
    use std::sync::Arc as StdArc;

    fn env_lock() -> &'static std::sync::Mutex<()> {
        crate::session::snapshot_paths::test_home::env_lock()
    }

    fn with_snapshots_root<R>(root: &Path, f: impl FnOnce() -> R) -> R {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // SAFETY: serialized by the shared test-home lock for tests that mutate
        // this var.
        unsafe {
            std::env::set_var(SNAPSHOTS_ROOT_ENV, root);
        }
        let out = f();
        unsafe {
            std::env::remove_var(SNAPSHOTS_ROOT_ENV);
        }
        out
    }

    #[test]
    fn workspace_snapshot_lock_is_shared_per_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let a = workspace_snapshot_lock(ws);
        let b = workspace_snapshot_lock(ws);
        assert!(
            StdArc::ptr_eq(&a, &b),
            "same workspace must resolve to a single shared lock"
        );

        let other = tempfile::tempdir().unwrap();
        let c = workspace_snapshot_lock(other.path());
        assert!(
            !StdArc::ptr_eq(&a, &c),
            "distinct workspaces must have distinct locks"
        );
    }

    #[test]
    fn open_rejects_in_workspace_snapshots_dir() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let bad = ws.join(".litecode").join("snapshots");
        std::fs::create_dir_all(&bad).unwrap();
        let err = match open_or_init_repo(ws, &bad) {
            Err(e) => e,
            Ok(_) => panic!("expected isolation error for in-workspace snapshots dir"),
        };
        assert!(
            err.to_string().contains("must not be inside workspace"),
            "got: {err}"
        );
    }

    #[test]
    fn snapshot_path_excluded_skips_dir_entries_and_heavy_trees() {
        assert!(snapshot_path_excluded("docs/"));
        assert!(snapshot_path_excluded("Library/ArtifactDB"));
        assert!(snapshot_path_excluded("node_modules/leftpad/index.js"));
        assert!(snapshot_path_excluded(".litecode/sessions.db"));
        assert!(snapshot_path_excluded("Assets/Tex/hero.png"));
        assert!(snapshot_path_excluded("Assets/Foo.cs.meta"));
        assert!(snapshot_path_excluded("Foo.csproj"));
        assert!(!snapshot_path_excluded("Assets/Scripts/Foo.cs"));
        assert!(!snapshot_path_excluded("Assets/X.prefab"));
        assert!(!snapshot_path_excluded("docs/README.md"));
    }

    #[test]
    fn snapshot_track_skips_nested_git_dir_entry() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            // Nested checkout / submodule-shaped tree: directory with its own .git.
            let nested = ws.join("docs");
            std::fs::create_dir_all(&nested).unwrap();
            std::fs::write(nested.join(".git"), "gitdir: ../.git/modules/docs\n").unwrap();
            std::fs::write(nested.join("README.md"), "nested").unwrap();

            let snaps = snapshots_dir_for_workspace(&ws);
            let r = snapshot_track(&ws, &snaps, "sess", 0);
            assert!(
                r.is_ok(),
                "nested .git must not yield invalid path 'docs/': {r:?}"
            );
            assert!(snapshot_exists(&snaps, "sess", 0));
        });
    }

    #[test]
    fn snapshot_track_recovers_from_index_lock() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);
            init_snapshot_repo(&ws, &snaps).unwrap();
            std::fs::write(snaps.join("index.lock"), b"stale").unwrap();

            let r = snapshot_track(&ws, &snaps, "sess", 0);
            assert!(
                r.is_ok(),
                "index.lock must be cleared on Locked retry: {r:?}"
            );
            assert!(!snaps.join("index.lock").exists());
            assert!(snapshot_exists(&snaps, "sess", 0));
        });
    }

    #[test]
    fn warm_then_incremental_edit_restore() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            std::fs::write(ws.join("b.txt"), "keep").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);

            warm_snapshot_repo(&ws, &snaps).unwrap();
            let repo = git2::Repository::open_bare(&snaps).unwrap();
            assert!(
                !repo.index().unwrap().is_empty(),
                "warm must populate index"
            );

            snapshot_track(&ws, &snaps, "sess", 0).unwrap();
            std::fs::write(ws.join("a.txt"), "changed").unwrap();
            let patch = snapshot_record_patch(&ws, &snaps, "sess", 0).unwrap();
            assert!(!patch.track_failed);
            assert_eq!(patch.files, vec!["a.txt".to_string()]);

            let outcome = snapshot_restore(&ws, &snaps, "sess", 0, 1).unwrap();
            assert!(matches!(outcome, RestoreOutcome::Restored { .. }));
            assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "v0");
            assert_eq!(std::fs::read_to_string(ws.join("b.txt")).unwrap(), "keep");
        });
    }

    #[test]
    fn incremental_tracks_delete_and_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("keep.txt"), "k").unwrap();
            std::fs::write(ws.join("gone.txt"), "g").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);
            warm_snapshot_repo(&ws, &snaps).unwrap();
            snapshot_track(&ws, &snaps, "sess", 0).unwrap();

            std::fs::remove_file(ws.join("gone.txt")).unwrap();
            std::fs::write(ws.join("new.txt"), "n").unwrap();
            let patch = snapshot_record_patch(&ws, &snaps, "sess", 0).unwrap();
            assert!(!patch.track_failed);
            assert!(patch.files.iter().any(|p| p == "gone.txt"));
            assert!(patch.files.iter().any(|p| p == "new.txt"));

            let outcome = snapshot_restore(&ws, &snaps, "sess", 0, 1).unwrap();
            assert!(matches!(outcome, RestoreOutcome::Restored { .. }));
            assert!(ws.join("gone.txt").exists());
            assert!(!ws.join("new.txt").exists());
            assert_eq!(std::fs::read_to_string(ws.join("keep.txt")).unwrap(), "k");
        });
    }

    #[test]
    fn incremental_skips_excluded_paths() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("ok.txt"), "v0").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);
            warm_snapshot_repo(&ws, &snaps).unwrap();
            snapshot_track(&ws, &snaps, "sess", 0).unwrap();

            std::fs::create_dir_all(ws.join("node_modules").join("pkg")).unwrap();
            std::fs::write(ws.join("node_modules").join("pkg").join("index.js"), "x").unwrap();
            std::fs::write(ws.join("photo.png"), b"\x89PNG").unwrap();
            std::fs::write(ws.join("ok.txt"), "v1").unwrap();

            let patch = snapshot_record_patch(&ws, &snaps, "sess", 0).unwrap();
            assert!(!patch.track_failed);
            assert_eq!(patch.files, vec!["ok.txt".to_string()]);
            assert!(!patch.files.iter().any(|p| p.contains("node_modules")));
            assert!(!patch.files.iter().any(|p| p.ends_with(".png")));
        });
    }

    #[test]
    fn snapshot_track_skips_gitignore_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("keep.txt"), "k").unwrap();
            std::fs::write(ws.join(".gitignore"), "secret/\nweb/remotion/\n").unwrap();
            std::fs::create_dir_all(ws.join("secret")).unwrap();
            std::fs::write(ws.join("secret").join("a.txt"), "nope").unwrap();
            std::fs::create_dir_all(ws.join("web").join("remotion")).unwrap();
            std::fs::write(ws.join("web").join("remotion").join("Full.tsx"), "x").unwrap();

            let snaps = snapshots_dir_for_workspace(&ws);
            let r = snapshot_track(&ws, &snaps, "sess", 0);
            assert!(r.is_ok(), "ignored dirs must not abort git add: {r:?}");
            assert!(snapshot_exists(&snaps, "sess", 0));
        });
    }

    #[test]
    fn snapshot_track_drops_paths_after_gitignore_change() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("keep.txt"), "k").unwrap();
            std::fs::create_dir_all(ws.join("secret")).unwrap();
            std::fs::write(ws.join("secret").join("a.txt"), "tracked-then-ignored").unwrap();

            let snaps = snapshots_dir_for_workspace(&ws);
            snapshot_track(&ws, &snaps, "sess", 0).unwrap();

            std::fs::write(ws.join(".gitignore"), "secret/\n").unwrap();
            std::fs::write(ws.join("secret").join("a.txt"), "changed").unwrap();
            std::fs::write(ws.join("keep.txt"), "k2").unwrap();

            let r = snapshot_track(&ws, &snaps, "sess", 1);
            assert!(
                r.is_ok(),
                "gitignore change must unstage ignored paths, not fail add: {r:?}"
            );
            assert!(snapshot_exists(&snaps, "sess", 1));

            let repo = git2::Repository::open_bare(&snaps).unwrap();
            let oid = repo.refname_to_id("refs/snapshots/sess/1").unwrap();
            let tree = repo.find_tree(oid).unwrap();
            assert!(
                tree.get_path(std::path::Path::new("keep.txt")).is_ok(),
                "tracked keep.txt must remain"
            );
            assert!(
                tree.get_path(std::path::Path::new("secret/a.txt")).is_err(),
                "gitignore change must drop secret/ from the snapshot tree"
            );
        });
    }

    #[test]
    fn sequential_snapshot_track_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            let _ = std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&ws)
                .status();
            let snaps = snapshots_dir_for_workspace(&ws);
            std::fs::write(ws.join("file.txt"), "v0").unwrap();
            init_snapshot_repo(&ws, &snaps).unwrap();
            let alt = snaps.join("objects").join("info").join("alternates");
            assert!(
                alt.is_file(),
                "project git should seed read-only alternates"
            );
            let alt_body = std::fs::read_to_string(&alt).unwrap();
            assert!(
                alt_body.contains("objects"),
                "alternates should point at project objects, got: {alt_body}"
            );
            // Seed must never point inside the workspace tree itself (except .git/objects).
            for line in alt_body.lines().filter(|l| !l.is_empty()) {
                let p = PathBuf::from(line);
                assert!(p.exists(), "alternate target must exist: {}", p.display());
            }
            for i in 0..4u64 {
                let r = snapshot_track(&ws, &snaps, "sess", i as i64);
                assert!(r.is_ok(), "sequential snapshot_track {i} failed: {r:?}");
            }
            assert_eq!(
                snaps.parent().map(Path::as_os_str),
                Some(snap_root.path().as_os_str()),
                "snaps={} should be directly under snap_root={}",
                snaps.display(),
                snap_root.path().display()
            );
            assert!(!path_is_under(&snaps, &ws));
        });
    }

    #[test]
    fn file_level_restore_only_touches_patch_paths() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            let _ = std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&ws)
                .status();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            std::fs::write(ws.join("user.txt"), "keep-me").unwrap();
            let _ = std::process::Command::new("git")
                .current_dir(&ws)
                .args(["add", "a.txt", "user.txt"])
                .status();
            let _ = std::process::Command::new("git")
                .current_dir(&ws)
                .args([
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "user.name=t",
                    "commit",
                    "-qm",
                    "i",
                ])
                .status();
            let head_before = std::fs::read_to_string(ws.join(".git").join("HEAD")).unwrap();

            let snaps = snapshots_dir_for_workspace(&ws);
            snapshot_track(&ws, &snaps, "sess", 0).unwrap();

            // Agent-like edits (will be recorded in patch).
            std::fs::write(ws.join("a.txt"), "changed").unwrap();
            std::fs::write(ws.join("b.txt"), "new").unwrap();
            let patch = snapshot_record_patch(&ws, &snaps, "sess", 0).unwrap();
            assert!(!patch.track_failed);
            assert!(patch.files.iter().any(|p| p == "a.txt"));
            assert!(patch.files.iter().any(|p| p == "b.txt"));

            // User edit outside the agent patch (after record — simulates concurrent edit
            // that would not be in patch if recorded after; here we edit after patch so
            // restore must leave user.txt alone even if we also change it now).
            std::fs::write(ws.join("user.txt"), "user-edited").unwrap();

            let outcome = snapshot_restore(&ws, &snaps, "sess", 0, 1).unwrap();
            assert!(matches!(outcome, RestoreOutcome::Restored { .. }));

            assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "v0");
            assert!(
                !ws.join("b.txt").exists(),
                "new file in patch must be deleted"
            );
            assert_eq!(
                std::fs::read_to_string(ws.join("user.txt")).unwrap(),
                "user-edited",
                "files outside patch union must survive"
            );
            assert!(ws.join(".git").join("HEAD").is_file());
            assert_eq!(
                std::fs::read_to_string(ws.join(".git").join("HEAD")).unwrap(),
                head_before
            );
        });
    }

    #[test]
    fn restore_without_patch_is_nothing_to_revert() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);
            snapshot_track(&ws, &snaps, "sess", 0).unwrap();
            std::fs::write(ws.join("a.txt"), "changed").unwrap();
            // No snapshot_record_patch → empty union → nothing to revert.
            let outcome = snapshot_restore(&ws, &snaps, "sess", 0, 1).unwrap();
            assert_eq!(outcome, RestoreOutcome::NothingToRevert);
            assert_eq!(
                std::fs::read_to_string(ws.join("a.txt")).unwrap(),
                "changed"
            );
        });
    }

    #[test]
    fn restore_missing_track_ref_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);
            init_snapshot_repo(&ws, &snaps).unwrap();
            // Never tracked — no refs/snapshots/.../0
            let outcome = snapshot_restore(&ws, &snaps, "sess", 0, 1).unwrap();
            assert_eq!(
                outcome,
                RestoreOutcome::Unavailable {
                    reason: RestoreUnavailable::MissingTrackRef,
                }
            );
        });
    }

    #[test]
    fn record_patch_missing_ref_marks_track_failed() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);
            init_snapshot_repo(&ws, &snaps).unwrap();
            let patch = snapshot_record_patch(&ws, &snaps, "sess", 0).unwrap();
            assert!(patch.track_failed);
            assert!(patch.files.is_empty());
            let stored = read_patch_file(&snaps, "sess", 0).unwrap();
            assert_eq!(stored.status, PatchStatus::TrackFailed);
        });
    }

    #[test]
    fn restore_track_failed_patch_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);
            // Track succeeds, then simulate a later failed track by writing marker
            // without a corresponding successful patch for a higher anchor — here we
            // track k=0, then write track_failed for k=0 directly (as record would).
            snapshot_track(&ws, &snaps, "sess", 0).unwrap();
            write_patch(&snaps, "sess", 0, &[], PatchStatus::TrackFailed).unwrap();
            std::fs::write(ws.join("a.txt"), "changed").unwrap();
            let outcome = snapshot_restore(&ws, &snaps, "sess", 0, 1).unwrap();
            assert_eq!(
                outcome,
                RestoreOutcome::Unavailable {
                    reason: RestoreUnavailable::TrackFailed,
                }
            );
            assert_eq!(
                std::fs::read_to_string(ws.join("a.txt")).unwrap(),
                "changed"
            );
        });
    }

    #[test]
    fn restore_empty_ok_patch_is_nothing_to_revert() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);
            snapshot_track(&ws, &snaps, "sess", 0).unwrap();
            // Record with no file changes → empty ok patch.
            let patch = snapshot_record_patch(&ws, &snaps, "sess", 0).unwrap();
            assert!(!patch.track_failed);
            assert!(patch.files.is_empty());
            std::fs::write(ws.join("a.txt"), "user-only-edit").unwrap();
            let outcome = snapshot_restore(&ws, &snaps, "sess", 0, 1).unwrap();
            assert_eq!(outcome, RestoreOutcome::NothingToRevert);
            assert_eq!(
                std::fs::read_to_string(ws.join("a.txt")).unwrap(),
                "user-only-edit"
            );
        });
    }

    #[test]
    fn max_file_revert_k_is_highest_nonempty_ok_patch() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("a.txt"), "v0").unwrap();
            let snaps = snapshots_dir_for_workspace(&ws);
            assert_eq!(max_file_revert_k(&snaps, "sess"), None);

            snapshot_track(&ws, &snaps, "sess", 0).unwrap();
            snapshot_record_patch(&ws, &snaps, "sess", 0).unwrap();
            assert_eq!(max_file_revert_k(&snaps, "sess"), None);

            snapshot_track(&ws, &snaps, "sess", 1).unwrap();
            std::fs::write(ws.join("a.txt"), "v1").unwrap();
            snapshot_record_patch(&ws, &snaps, "sess", 1).unwrap();
            assert_eq!(max_file_revert_k(&snaps, "sess"), Some(1));

            write_patch(&snaps, "sess", 2, &[], PatchStatus::TrackFailed).unwrap();
            assert_eq!(max_file_revert_k(&snaps, "sess"), Some(1));
        });
    }

    #[test]
    fn session_delete_uses_external_snapshots_not_data_root() {
        let dir = tempfile::tempdir().unwrap();
        let snap_root = tempfile::tempdir().unwrap();
        with_snapshots_root(snap_root.path(), || {
            let ws = dir.path().to_path_buf();
            std::fs::write(ws.join("f.txt"), "x").unwrap();
            let litecode = ws.join(".litecode");
            std::fs::create_dir_all(&litecode).unwrap();
            let db = litecode.join("sessions.db");
            let session = crate::session::store::Session::open(
                db.to_str().unwrap(),
                ws.to_str().unwrap(),
                "default",
                None,
            )
            .unwrap();
            let sid = session.id.clone();
            let snaps = snapshots_dir_for_workspace(&ws);
            snapshot_track(&ws, &snaps, &sid, 0).unwrap();
            assert!(snapshot_exists(&snaps, &sid, 0));
            drop(session);

            crate::session::store::Session::delete(db.to_str().unwrap(), &sid).unwrap();

            assert!(
                !litecode.join("snapshots").exists(),
                "delete must not create data_root/snapshots"
            );
            assert!(
                !snapshot_exists(&snaps, &sid, 0),
                "external snapshot refs must be removed"
            );
        });
    }

    #[test]
    fn per_workspace_lock_serialises_concurrent_critical_sections() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();

        let inside = Arc::new(AtomicUsize::new(0));
        let max_inside = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let ws = ws.clone();
            let inside = inside.clone();
            let max_inside = max_inside.clone();
            handles.push(std::thread::spawn(move || {
                let lk = workspace_snapshot_lock(&ws);
                let _lock = lk.lock().expect("snapshot lock poisoned");
                let cur = inside.fetch_add(1, Ordering::SeqCst) + 1;
                max_inside.fetch_max(cur, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(2));
                inside.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            max_inside.load(Ordering::SeqCst),
            1,
            "per-workspace mutex must serialise critical sections (same workspace)"
        );

        let other = tempfile::tempdir().unwrap();
        let ws2 = other.path().to_path_buf();
        let combined = Arc::new(AtomicUsize::new(0));
        let max_combined = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let ws = ws.clone();
            let combined = combined.clone();
            let max_combined = max_combined.clone();
            handles.push(std::thread::spawn(move || {
                let lk = workspace_snapshot_lock(&ws);
                let _lock = lk.lock().expect("snapshot lock poisoned");
                let cur = combined.fetch_add(1, Ordering::SeqCst) + 1;
                max_combined.fetch_max(cur, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(2));
                combined.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for _ in 0..4 {
            let ws2 = ws2.clone();
            let combined = combined.clone();
            let max_combined = max_combined.clone();
            handles.push(std::thread::spawn(move || {
                let lk = workspace_snapshot_lock(&ws2);
                let _lock = lk.lock().expect("snapshot lock poisoned");
                let cur = combined.fetch_add(1, Ordering::SeqCst) + 1;
                max_combined.fetch_max(cur, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(2));
                combined.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let peak = max_combined.load(Ordering::SeqCst);
        assert!(
            peak >= 2,
            "distinct workspaces must have independent locks (combined concurrency was {peak}, expected >= 2)"
        );
    }

    #[test]
    fn snapshot_restore_swallowed_anchor_returns_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_path_buf();
        let snaps = snapshots_dir_for_workspace(&ws);
        // A swallowed anchor (k >= visible user detail count) fails closed before any
        // repo/track work — the guard at the top of `snapshot_restore`.
        let err = snapshot_restore(&ws, &snaps, "sess", 3, 1).unwrap_err();
        assert!(
            matches!(err, crate::types::LitecodeError::InvalidRevertAnchor(_)),
            "swallowed anchor must yield InvalidRevertAnchor, got {err:?}"
        );
    }
}
