//! Workspace drift since the newest knowledge file was written.
//!
//! Nothing is stored. The section is omitted when git cannot answer, or when
//! the change is still under the threshold.

use std::path::Path;
use std::time::UNIX_EPOCH;

use super::corpus::Corpus;
use crate::workspace::{self, WorktreeDrift};

const FILE_THRESHOLD: u32 = 20;
const LINE_THRESHOLD: u32 = 400;

pub fn section(workspace: &Path, corpus: &Corpus) -> Option<String> {
    let root = corpus.root.as_deref()?;
    let modified = corpus.latest_modified()?;
    let since = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let drift = workspace::drift_since(workspace, since, &[".litecode", root]).ok()??;
    body(drift)
}

pub fn body(drift: WorktreeDrift) -> Option<String> {
    let lines = drift.added.saturating_add(drift.deleted);
    if drift.files < FILE_THRESHOLD && lines < LINE_THRESHOLD {
        return None;
    }
    let files_label = if drift.files == 1 { "file" } else { "files" };
    Some(format!(
        "> The workspace has changed a lot since the newest node file was written. Tell the user and ask before reorganizing the knowledge base.\n**{} {files_label} changed · +{} lines · −{} lines** (excludes the knowledge root and `.litecode/`)",
        drift.files, drift.added, drift.deleted
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::time::{Duration, SystemTime};

    use crate::config::git_install::find_git_exe;

    fn init_repo(dir: &Path) {
        let git = find_git_exe().expect("git");
        let status = Command::new(&git)
            .args(["-c", "init.defaultBranch=main", "init"])
            .current_dir(dir)
            .status()
            .expect("git init");
        assert!(status.success());
        let cfg = |key: &str, val: &str| {
            assert!(
                Command::new(&git)
                    .args(["config", key, val])
                    .current_dir(dir)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        cfg("user.email", "test@litecode.local");
        cfg("user.name", "Litecode Test");
        cfg("commit.gpgsign", "false");
        cfg("core.autocrlf", "false");
        cfg("core.eol", "lf");
        fs::write(dir.join("a.txt"), "base\n").unwrap();
        assert!(
            Command::new(&git)
                .args(["add", "a.txt"])
                .current_dir(dir)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new(&git)
                .args(["commit", "-m", "init"])
                .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00")
                .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00")
                .current_dir(dir)
                .status()
                .unwrap()
                .success()
        );
    }

    fn touch_knowledge(dir: &Path) {
        let root = dir.join(".litecode").join("knowledge");
        fs::create_dir_all(&root).unwrap();
        let file = root.join("seq.md");
        fs::write(
            &file,
            "```node\nnode : seq\nstatus : enabled\nsummary : \n```\n",
        )
        .unwrap();
        let mtime = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        let handle = fs::OpenOptions::new().write(true).open(&file).unwrap();
        handle.set_modified(mtime).unwrap();
    }

    #[test]
    fn reminder_follows_file_and_line_thresholds() {
        if find_git_exe().is_none() {
            return;
        }
        let below = tempfile::tempdir().unwrap();
        init_repo(below.path());
        touch_knowledge(below.path());
        fs::write(
            below.path().join("a.txt"),
            format!("base\n{}", "x\n".repeat(10)),
        )
        .unwrap();
        let corpus = Corpus::load(below.path());
        let drift = workspace::drift_since(
            below.path(),
            1_600_000_000,
            &[".litecode", ".litecode/knowledge"],
        )
        .unwrap()
        .expect("baseline");
        assert_eq!(drift.files, 1);
        assert_eq!(drift.added, 10);
        assert!(section(below.path(), &corpus).is_none());

        let files = tempfile::tempdir().unwrap();
        init_repo(files.path());
        touch_knowledge(files.path());
        for index in 0..20 {
            fs::write(files.path().join(format!("n{index}.txt")), "n\n").unwrap();
        }
        let corpus = Corpus::load(files.path());
        let text = section(files.path(), &corpus).expect("file threshold");
        assert!(text.contains("20 files changed"));
        assert!(text.contains("+0 lines"));
        assert!(text.contains("excludes the knowledge root"));

        let lines = tempfile::tempdir().unwrap();
        init_repo(lines.path());
        touch_knowledge(lines.path());
        fs::write(
            lines.path().join("a.txt"),
            format!("base\n{}", "x\n".repeat(400)),
        )
        .unwrap();
        let corpus = Corpus::load(lines.path());
        let text = section(lines.path(), &corpus).expect("line threshold");
        assert!(text.contains("1 file changed"));
        assert!(text.contains("+400 lines"));
        assert!(text.contains("−0 lines"));
    }
}
