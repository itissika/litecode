use crate::reminder::kinds::{CustomToolSettledBody, CustomToolSettledEntry, Reminder};

pub(crate) fn custom_tool_settled(settled: &[CustomToolSettledEntry]) -> Option<Reminder> {
    if settled.is_empty() {
        return None;
    }
    let mut text =
        String::from("source: custom_tool\nThe following background custom tool jobs settled.\n");
    text.push_str(&format!("settled: {}\n", settled.len()));
    for entry in settled {
        text.push_str("---\n");
        text.push_str(&format!(
            "job_id: {}\ncall_id: {}\ntool: {}\nstatus: {}\n",
            entry.job_id, entry.call_id, entry.tool_name, entry.status
        ));
        if !entry.detail.is_empty() {
            text.push_str(&format!("detail: {}\n", entry.detail));
        }
    }
    Some(Reminder::CustomToolSettled(CustomToolSettledBody {
        settled: settled.to_vec(),
        text: text.trim_end().to_string(),
    }))
}
