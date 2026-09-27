use crate::reminder::kinds::{Reminder, SettledChild, SubagentSettledBody};

pub(crate) fn subagent_settled(settled: &[SettledChild], detail: &str) -> Option<Reminder> {
    if settled.is_empty() {
        return None;
    }
    let mut text =
        String::from("source: subagent\nThe following background child session turns settled.\n");
    let detail = detail.trim();
    if detail.is_empty() {
        text.push_str(&format!("status: settled\nsettled: {}\n", settled.len()));
        for child in settled {
            text.push_str("---\n");
            text.push_str(&format!(
                "child_session_id: {}\nturn_id: {}\nreason: {}\n",
                child.child_session_id, child.turn_id, child.reason
            ));
            if !child.agent.is_empty() {
                text.push_str(&format!("agent: {}\n", child.agent));
            }
        }
    } else {
        text.push_str(detail);
        if !detail.ends_with('\n') {
            text.push('\n');
        }
    }
    Some(Reminder::SubagentSettled(SubagentSettledBody {
        settled: settled.to_vec(),
        text: text.trim_end().to_string(),
    }))
}
