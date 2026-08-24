//! Shared agent-facing bash job status text (running list + reminders).

use std::path::Path;

use crate::terminal::{ExitNotice, INLINE_HEAD, INLINE_TAIL, RunningJob, TeeCapture};

pub fn display_output_path(path: &Path, workspace_root: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(workspace_root) {
        return rel.to_string_lossy().replace('\\', "/");
    }
    if let (Ok(path_abs), Ok(root_abs)) = (
        crate::config::path::canon_abs(path),
        crate::config::path::canon_abs(workspace_root),
    ) && let Ok(rel) = path_abs.strip_prefix(&root_abs)
    {
        return rel.to_string_lossy().replace('\\', "/");
    }
    path.display().to_string().replace('\\', "/")
}

pub fn format_running_list(jobs: &[RunningJob], workspace_root: &Path) -> String {
    if jobs.is_empty() {
        return "running: 0\n".into();
    }
    let mut out = format!("running: {}\n", jobs.len());
    for j in jobs {
        let rel = display_output_path(&j.output_path, workspace_root);
        out.push_str(&format!("- {}  {}  ({rel})\n", j.id, j.command_preview));
    }
    out
}

pub fn guidance_line() -> &'static str {
    "Use wait_shell to wait. The output_file is the full log; grep/read it only if you need more than the inline window. kill_shell to stop."
}

pub fn format_running_status(
    bash_id: &str,
    output_path: &Path,
    workspace_root: &Path,
    jobs: &[RunningJob],
) -> String {
    let rel = display_output_path(output_path, workspace_root);
    let mut out = String::new();
    out.push_str("status: running\n");
    out.push_str(&format!("bash_id: {bash_id}\n"));
    out.push_str(&format!("output_file: {rel}\n"));
    out.push_str(&format_running_list(jobs, workspace_root));
    out.push_str(guidance_line());
    out.push('\n');
    out
}

pub fn format_exited_status(
    notice: &ExitNotice,
    workspace_root: &Path,
    jobs: &[RunningJob],
) -> String {
    let rel = display_output_path(&notice.output_path, workspace_root);
    let code = notice.exit_code.map(|c| c as i32).unwrap_or(-1);
    let mut out = String::new();
    out.push_str("status: exited\n");
    out.push_str(&format!("bash_id: {}\n", notice.bash_id));
    out.push_str(&format!("exit_code: {code}\n"));
    if notice.user_killed {
        out.push_str("stopped_by: user (Kill)\n");
    }
    out.push_str(&format!("output_file: {rel}\n"));
    if !notice.command_preview.is_empty() {
        out.push_str(&format!("command: {}\n", notice.command_preview));
    }
    out.push_str(&format_running_list(jobs, workspace_root));
    out.push_str(guidance_line());
    out.push('\n');
    out
}

pub fn format_waited_status(jobs: &[RunningJob], workspace_root: &Path) -> String {
    let mut out = String::from("status: waited\n");
    out.push_str(&format_running_list(jobs, workspace_root));
    out.push_str(guidance_line());
    out.push('\n');
    out
}

pub fn format_killed_status(
    bash_id: &str,
    exit_code: i32,
    output_path: Option<&Path>,
    workspace_root: &Path,
    jobs: &[RunningJob],
) -> String {
    let mut msg = format!("Terminated background task '{bash_id}' (exit_code: {exit_code}).\n");
    if let Some(path) = output_path {
        let rel = display_output_path(path, workspace_root);
        if !rel.is_empty() {
            msg.push_str(&format!("output_file: {rel}\n"));
        }
    }
    msg.push_str(&format_running_list(jobs, workspace_root));
    msg.push_str(guidance_line());
    msg.push('\n');
    msg
}

pub fn format_unknown_task(bash_id: &str, jobs: &[RunningJob], workspace_root: &Path) -> String {
    let mut msg = format!(
        "background task '{bash_id}' not found. It may have already exited or never existed.\n"
    );
    msg.push_str(&format_running_list(jobs, workspace_root));
    msg
}

pub fn format_exit_reminder(
    notices: &[ExitNotice],
    jobs: &[RunningJob],
    workspace_root: &Path,
) -> String {
    let mut inner = String::new();
    for n in notices {
        let rel = display_output_path(&n.output_path, workspace_root);
        let code = n.exit_code.map(|c| c as i32).unwrap_or(-1);
        if n.user_killed {
            inner.push_str(&format!(
                "The user stopped background bash {} (Kill).\nexit_code: {code}\noutput_file: {rel}\ncommand: {}\n",
                n.bash_id, n.command_preview
            ));
        } else {
            inner.push_str(&format!(
                "Background bash {} exited with code {code}.\noutput_file: {rel}\ncommand: {}\n",
                n.bash_id, n.command_preview
            ));
        }
    }
    inner.push_str(&format_running_list(jobs, workspace_root));
    format!("<system-reminder>\n{}</system-reminder>", inner.trim_end())
}

pub fn format_completed_view(
    capture: &TeeCapture,
    exit_code: Option<u32>,
    cancelled: bool,
    workspace_root: &Path,
) -> String {
    let rel = display_output_path(&capture.path, workspace_root);
    let mut out = String::new();
    if cancelled {
        out.push_str("status: cancelled\n");
    } else {
        let code = exit_code.map(|c| c as i32).unwrap_or(-1);
        out.push_str(&format!("exit_code: {code}\n"));
    }

    if capture.frozen {
        out.push_str(&format!("bytes: {}\n", capture.total_bytes));
        out.push_str(&format!("output_file: {rel}\n"));
        if capture.truncated_on_disk {
            out.push_str("truncated_on_disk: true\n");
        }
        out.push_str(&format!(
            "[head {INLINE_HEAD}B + tail {INLINE_TAIL}B of {} bytes. output_file has the full log — grep/read it only if this window is not enough. Do not re-run.]\n",
            capture.total_bytes
        ));
        if cancelled {
            out.push_str("Command cancelled.\n");
        }
        out.push_str("\n--- head ---\n");
        out.push_str(&capture.head);
        if !capture.head.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("--- tail ---\n");
        out.push_str(&capture.tail);
        if !capture.tail.is_empty() && !capture.tail.ends_with('\n') {
            out.push('\n');
        }
    } else {
        if !capture.head.is_empty() {
            out.push_str(&capture.head);
            if !capture.head.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{ExitNotice, RunningJob, TeeCapture};
    use std::path::PathBuf;

    fn fixture(root: &Path, id: &str, cmd: &str) -> (PathBuf, RunningJob) {
        let output_path = root
            .join(".litecode")
            .join("bash")
            .join(format!("{id}.output"));
        let job = RunningJob {
            id: id.into(),
            command_preview: cmd.into(),
            output_path: output_path.clone(),
        };
        (output_path, job)
    }

    #[test]
    fn running_status_field_order_and_guidance() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, job) = fixture(root, "bg_aaa", "cargo test");
        let jobs = vec![job];
        let got = format_running_status("bg_aaa", &path, root, &jobs);
        let rel = display_output_path(&path, root);
        assert_eq!(
            got,
            format!(
                "status: running\nbash_id: bg_aaa\noutput_file: {rel}\n{}{}\n",
                format_running_list(&jobs, root),
                guidance_line()
            )
        );
        assert_eq!(rel, ".litecode/bash/bg_aaa.output");
        assert!(got.contains("- bg_aaa  cargo test  (.litecode/bash/bg_aaa.output)\n"));
    }

    #[test]
    fn exited_waited_killed_unknown_and_reminder_share_running_list() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (path, leftover) = fixture(root, "bg_bbb", "npm install");
        let jobs = vec![leftover];
        let notice = ExitNotice {
            bash_id: "bg_aaa".into(),
            session_id: "s1".into(),
            exit_code: Some(0),
            output_path: root.join(".litecode").join("bash").join("bg_aaa.output"),
            command_preview: "cargo test".into(),
            user_killed: false,
        };

        let exited = format_exited_status(&notice, root, &jobs);
        assert!(exited.starts_with("status: exited\nbash_id: bg_aaa\nexit_code: 0\n"));
        assert!(exited.contains("command: cargo test\n"));
        assert!(exited.contains(&format_running_list(&jobs, root)));
        assert!(exited.contains(guidance_line()));

        let waited = format_waited_status(&jobs, root);
        assert!(waited.starts_with("status: waited\n"));
        assert!(waited.contains(&format_running_list(&jobs, root)));
        assert!(waited.contains(guidance_line()));

        let killed = format_killed_status("bg_aaa", -1, Some(&path), root, &jobs);
        assert!(killed.starts_with("Terminated background task 'bg_aaa' (exit_code: -1).\n"));
        assert!(killed.contains("output_file: .litecode/bash/bg_bbb.output\n"));
        assert!(killed.contains(&format_running_list(&jobs, root)));
        assert!(killed.contains(guidance_line()));

        let unknown = format_unknown_task("missing", &jobs, root);
        assert!(unknown.starts_with(
            "background task 'missing' not found. It may have already exited or never existed.\n"
        ));
        assert!(unknown.contains(&format_running_list(&jobs, root)));
        assert!(!unknown.contains(guidance_line()));

        let reminder = format_exit_reminder(&[notice.clone()], &jobs, root);
        assert!(reminder.starts_with("<system-reminder>\n"));
        assert!(reminder.ends_with("</system-reminder>"));
        assert!(reminder.contains("Background bash bg_aaa exited with code 0."));
        assert!(reminder.contains(format_running_list(&jobs, root).trim_end()));
        assert!(!reminder.contains("status: exited"));

        let mut user_stopped = notice.clone();
        user_stopped.user_killed = true;
        user_stopped.exit_code = Some(143);
        let exited_kill = format_exited_status(&user_stopped, root, &jobs);
        assert!(exited_kill.contains("stopped_by: user (Kill)\n"));
        assert!(exited_kill.contains("exit_code: 143\n"));
        let killed_reminder = format_exit_reminder(&[user_stopped], &jobs, root);
        assert!(
            killed_reminder
                .contains("The user stopped background bash bg_aaa (Kill).\nexit_code: 143\n")
        );
        assert!(!killed_reminder.contains("Background bash bg_aaa exited"));
    }

    #[test]
    fn completed_small_view_omits_output_file() {
        let dir = tempfile::tempdir().unwrap();
        let capture = TeeCapture {
            path: dir
                .path()
                .join(".litecode")
                .join("bash")
                .join("bg_x.output"),
            head: "hello\n".into(),
            tail: String::new(),
            frozen: false,
            total_bytes: 6,
            truncated_on_disk: false,
        };
        let got = format_completed_view(&capture, Some(0), false, dir.path());
        assert_eq!(got, "exit_code: 0\nhello\n");
        assert!(!got.contains("output_file"));
    }

    #[test]
    fn completed_frozen_view_points_at_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(".litecode")
            .join("bash")
            .join("bg_x.output");
        let capture = TeeCapture {
            path,
            head: "HEAD\n".into(),
            tail: "TAIL\n".into(),
            frozen: true,
            total_bytes: 99,
            truncated_on_disk: true,
        };
        let got = format_completed_view(&capture, Some(0), false, dir.path());
        assert_eq!(
            got,
            format!(
                "exit_code: 0\nbytes: 99\noutput_file: .litecode/bash/bg_x.output\ntruncated_on_disk: true\n[head {INLINE_HEAD}B + tail {INLINE_TAIL}B of 99 bytes. output_file has the full log — grep/read it only if this window is not enough. Do not re-run.]\n\n--- head ---\nHEAD\n--- tail ---\nTAIL\n"
            )
        );
    }
}
