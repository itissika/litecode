use chrono::Local;

use crate::reminder::engine::{SeamCtx, SpineReminderView};
use crate::reminder::kinds::{EnvBody, Reminder, ReminderKind};

pub(crate) fn env(ctx: &SeamCtx, view: &SpineReminderView) -> Option<Reminder> {
    let now = Local::now();
    let cwd = ctx.cwd.display().to_string().replace('\\', "/");
    let os = std::env::consts::OS.to_string();
    let date = now.format("%Y-%m-%d").to_string();
    let timezone = now.format("%:z").to_string();
    let reminder = Reminder::Env(EnvBody {
        text: format!("Environment: cwd {cwd}; os {os}; date {date}; timezone {timezone}."),
        cwd,
        os,
        date,
        timezone,
    });
    match view.latest(ReminderKind::Env) {
        Some(previous) if previous.same_snapshot(&reminder) => None,
        _ => Some(reminder),
    }
}
