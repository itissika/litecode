use crate::reminder::kinds::{BashExitBody, BashExitEntry, Reminder, RunningBash};

pub(crate) fn bash_exit(exits: &[BashExitEntry], running: &[RunningBash]) -> Option<Reminder> {
    if exits.is_empty() {
        return None;
    }
    let mut inner = String::new();
    for notice in exits {
        if notice.killed {
            inner.push_str(&format!(
                "The user stopped background bash {} (Kill).\nexit_code: {}\noutput_file: {}\ncommand: {}\n",
                notice.job_id, notice.exit_code, notice.output_file, notice.command
            ));
        } else {
            inner.push_str(&format!(
                "Background bash {} exited with code {}.\noutput_file: {}\ncommand: {}\n",
                notice.job_id, notice.exit_code, notice.output_file, notice.command
            ));
        }
    }
    inner.push_str(&running_list(running));
    Some(Reminder::BashExit(BashExitBody {
        exits: exits.to_vec(),
        running: running.to_vec(),
        text: inner.trim_end().to_string(),
    }))
}

fn running_list(jobs: &[RunningBash]) -> String {
    if jobs.is_empty() {
        return "running: 0\n".into();
    }
    let mut out = format!("running: {}\n", jobs.len());
    for job in jobs {
        out.push_str(&format!(
            "- {}  {}  ({})\n",
            job.job_id, job.command, job.output_file
        ));
    }
    out
}
