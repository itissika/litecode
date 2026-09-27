use crate::reminder::kinds::{FilesChangedBody, Reminder};

const PATH_CAP: usize = 20;

pub(crate) fn files_changed(paths: &[String]) -> Option<Reminder> {
    if paths.is_empty() {
        return None;
    }
    let more = paths.len().saturating_sub(PATH_CAP);
    let shown: Vec<String> = paths.iter().take(PATH_CAP).cloned().collect();
    let mut lines = vec!["Files changed outside this session:".to_string()];
    for path in &shown {
        lines.push(format!("- {path}"));
    }
    if more > 0 {
        lines.push(format!("and {more} more"));
    }
    Some(Reminder::FilesChanged(FilesChangedBody {
        paths: shown,
        more,
        text: lines.join("\n"),
    }))
}
