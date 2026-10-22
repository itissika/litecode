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
    
    
    
    
    

    

}
