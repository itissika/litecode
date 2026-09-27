//! One-shot rewrite of pre-reminder-engine rows.
//!
//! `seq` is never touched. Unparseable text keeps an empty structured list and
//! the original inner text.

use crate::reminder::kinds::{
    BashExitBody, BashExitEntry, PlanChangedBody, Reminder, ReminderKind, RunningBash,
    SettledChild, SubagentSettledBody,
};
use crate::reminder::{SYSTEM_REMINDER_CLOSE, SYSTEM_REMINDER_OPEN};
use crate::types::{Item, item_text_preview};

/// `(new_kind, new_body_json)` for a legacy reminder row. `None` when `kind`
/// is already current or not a reminder.
pub fn rewrite_reminder_row(kind: &str, body: &str) -> Option<(String, String)> {
    let text = item_plain_text(body);
    let inner = strip_reminder_tags(&text);
    match kind {
        "reminder/job_exit" => {
            if inner.contains("source: subagent") {
                let reminder = Reminder::SubagentSettled(parse_subagent(&inner));
                Some((
                    ReminderKind::SubagentSettled.wire().to_string(),
                    serde_json::to_string(&reminder).ok()?,
                ))
            } else {
                let reminder = Reminder::BashExit(parse_bash(&inner));
                Some((
                    ReminderKind::BashExit.wire().to_string(),
                    serde_json::to_string(&reminder).ok()?,
                ))
            }
        }
        "reminder/plan" => {
            let reminder = Reminder::PlanChanged(parse_plan(&inner));
            Some((
                ReminderKind::PlanChanged.wire().to_string(),
                serde_json::to_string(&reminder).ok()?,
            ))
        }
        _ => None,
    }
}

/// Drop a leading `<system-reminder>` block that sits after the summary label.
pub fn strip_embedded_reminder(summary: &str) -> String {
    let Some(open) = summary.find(SYSTEM_REMINDER_OPEN) else {
        return summary.to_string();
    };
    let after_open = &summary[open + SYSTEM_REMINDER_OPEN.len()..];
    let Some(close_rel) = after_open.find(SYSTEM_REMINDER_CLOSE) else {
        return summary.to_string();
    };
    let close = open + SYSTEM_REMINDER_OPEN.len() + close_rel + SYSTEM_REMINDER_CLOSE.len();
    let mut out = String::new();
    out.push_str(summary[..open].trim_end());
    let rest = summary[close..].trim_start_matches(['\n', '\r']);
    if !out.is_empty() && !rest.is_empty() {
        out.push('\n');
    }
    out.push_str(rest);
    out
}

fn item_plain_text(body: &str) -> String {
    serde_json::from_str::<Item>(body)
        .map(|item| item_text_preview(&item))
        .unwrap_or_else(|_| body.to_string())
}

fn strip_reminder_tags(text: &str) -> String {
    let text = text.trim();
    let Some(rest) = text.strip_prefix(SYSTEM_REMINDER_OPEN) else {
        return text.to_string();
    };
    rest.trim()
        .trim_end_matches(SYSTEM_REMINDER_CLOSE)
        .trim()
        .to_string()
}

fn parse_bash(inner: &str) -> BashExitBody {
    let mut exits = Vec::new();
    let mut running = Vec::new();
    let lines: Vec<&str> = inner.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if let Some(rest) = line.strip_prefix("Background bash ") {
            if let Some((job_id, tail)) = rest.split_once(" exited with code ") {
                let exit_code = tail.trim_end_matches('.').parse::<i32>().unwrap_or(-1);
                let (output_file, command, consumed) = following_fields(&lines[i + 1..]);
                exits.push(BashExitEntry {
                    job_id: job_id.trim().to_string(),
                    command,
                    exit_code,
                    killed: false,
                    output_file,
                });
                i += 1 + consumed;
                continue;
            }
        }
        if let Some(rest) = line.strip_prefix("The user stopped background bash ") {
            if let Some(job_id) = rest.strip_suffix(" (Kill).") {
                let (output_file, command, consumed) = following_fields(&lines[i + 1..]);
                let exit_code = lines[i + 1..]
                    .iter()
                    .find_map(|line| line.trim().strip_prefix("exit_code: "))
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(-1);
                exits.push(BashExitEntry {
                    job_id: job_id.trim().to_string(),
                    command,
                    exit_code,
                    killed: true,
                    output_file,
                });
                i += 1 + consumed;
                continue;
            }
        }
        if let Some(rest) = line.strip_prefix("- ") {
            let mut parts = rest.splitn(3, "  ");
            if let (Some(job_id), Some(command), Some(file)) =
                (parts.next(), parts.next(), parts.next())
            {
                running.push(RunningBash {
                    job_id: job_id.to_string(),
                    command: command.to_string(),
                    output_file: file
                        .trim()
                        .trim_matches(|c| c == '(' || c == ')')
                        .to_string(),
                });
            }
        }
        i += 1;
    }
    BashExitBody {
        exits,
        running,
        text: inner.to_string(),
    }
}

fn following_fields(lines: &[&str]) -> (String, String, usize) {
    let mut output_file = String::new();
    let mut command = String::new();
    let mut consumed = 0;
    for line in lines {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("output_file: ") {
            output_file = value.to_string();
            consumed += 1;
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("command: ") {
            command = value.to_string();
            consumed += 1;
            continue;
        }
        if trimmed.starts_with("exit_code: ") {
            consumed += 1;
            continue;
        }
        break;
    }
    (output_file, command, consumed)
}

fn parse_subagent(inner: &str) -> SubagentSettledBody {
    let mut settled = Vec::new();
    let mut current: Option<SettledChild> = None;
    for line in inner.lines() {
        let line = line.trim();
        if line == "---" {
            if let Some(child) = current.take() {
                settled.push(child);
            }
            continue;
        }
        if let Some(value) = line.strip_prefix("child_session_id: ") {
            if let Some(child) = current.take() {
                settled.push(child);
            }
            current = Some(SettledChild {
                child_session_id: value.to_string(),
                turn_id: String::new(),
                agent: String::new(),
                reason: String::new(),
            });
        } else if let Some(child) = current.as_mut() {
            if let Some(value) = line.strip_prefix("turn_id: ") {
                child.turn_id = value.to_string();
            } else if let Some(value) = line.strip_prefix("agent: ") {
                child.agent = value.to_string();
            } else if let Some(value) = line.strip_prefix("reason: ") {
                child.reason = value.to_string();
            }
        }
    }
    if let Some(child) = current {
        settled.push(child);
    }
    SubagentSettledBody {
        settled,
        text: inner.to_string(),
    }
}

fn parse_plan(inner: &str) -> PlanChangedBody {
    let relative_path = inner
        .lines()
        .find_map(|line| line.trim().strip_prefix("[Plan updated] "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("")
        .to_string();
    PlanChangedBody {
        relative_path,
        revision: String::new(),
        text: inner.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::user_text;

    fn item_body(text: &str) -> String {
        serde_json::to_string(&user_text(text)).unwrap()
    }

    #[test]
    fn job_exit_becomes_bash_exit_with_parsed_fields() {
        let text = "<system-reminder>\nBackground bash bg_a exited with code 3.\noutput_file: .litecode/bash/bg_a.output\ncommand: cargo test\nrunning: 0\n</system-reminder>";
        let (kind, body) = rewrite_reminder_row("reminder/job_exit", &item_body(text)).unwrap();
        assert_eq!(kind, "reminder/bash_exit");
        let reminder: Reminder = serde_json::from_str(&body).unwrap();
        let Reminder::BashExit(parsed) = reminder else {
            panic!("kind");
        };
        assert_eq!(parsed.exits.len(), 1);
        assert_eq!(parsed.exits[0].job_id, "bg_a");
        assert_eq!(parsed.exits[0].exit_code, 3);
        assert!(!parsed.exits[0].killed);
        assert!(parsed.text.contains("Background bash bg_a"));
    }

    #[test]
    fn job_exit_with_subagent_source_becomes_settled() {
        let text = "<system-reminder>\nsource: subagent\nchild_session_id: child-1\nturn_id: t1\nreason: completed\nagent: reviewer\n</system-reminder>";
        let (kind, body) = rewrite_reminder_row("reminder/job_exit", &item_body(text)).unwrap();
        assert_eq!(kind, "reminder/subagent_settled");
        let Reminder::SubagentSettled(parsed) = serde_json::from_str(&body).unwrap() else {
            panic!("kind");
        };
        assert_eq!(parsed.settled[0].child_session_id, "child-1");
        assert_eq!(parsed.settled[0].agent, "reviewer");
    }

    #[test]
    fn plan_row_keeps_path_and_empty_revision() {
        let text = "<system-reminder>\n[Plan updated] .litecode/plan/calm.md changed since you last read it.\n</system-reminder>";
        let (kind, body) = rewrite_reminder_row("reminder/plan", &item_body(text)).unwrap();
        assert_eq!(kind, "reminder/plan_changed");
        let Reminder::PlanChanged(parsed) = serde_json::from_str(&body).unwrap() else {
            panic!("kind");
        };
        assert_eq!(parsed.relative_path, ".litecode/plan/calm.md");
        assert!(parsed.revision.is_empty());
    }

    #[test]
    fn unparseable_bash_keeps_text_and_empty_exits() {
        let (kind, body) =
            rewrite_reminder_row("reminder/job_exit", &item_body("no structure here")).unwrap();
        assert_eq!(kind, "reminder/bash_exit");
        let Reminder::BashExit(parsed) = serde_json::from_str(&body).unwrap() else {
            panic!("kind");
        };
        assert!(parsed.exits.is_empty());
        assert_eq!(parsed.text, "no structure here");
    }

    #[test]
    fn strip_embedded_reminder_leaves_label_and_prose() {
        let summary = "[Conversation summary]\n<system-reminder>\nTodos:\n[~] a\n</system-reminder>\nkept prose";
        assert_eq!(
            strip_embedded_reminder(summary),
            "[Conversation summary]\nkept prose"
        );
    }

    #[test]
    fn rewrite_is_none_for_current_kinds() {
        assert!(rewrite_reminder_row("reminder/bash_exit", "{}").is_none());
    }
}
