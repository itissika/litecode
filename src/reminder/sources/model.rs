use crate::reminder::engine::{SeamCtx, SpineReminderView};
use crate::reminder::kinds::{ModelBody, Reminder, ReminderKind};

pub(crate) fn model(ctx: &SeamCtx, view: &SpineReminderView) -> Option<Reminder> {
    let model_ref = ctx.model_ref.trim();
    if model_ref.is_empty() {
        return None;
    }
    let text = match view.latest(ReminderKind::Model) {
        Some(Reminder::Model(previous)) if previous.model_ref == model_ref => return None,
        Some(Reminder::Model(previous)) => {
            format!("模型由 {} 切换为 {model_ref}", previous.model_ref)
        }
        _ => format!("当前模型为 {model_ref}"),
    };
    Some(Reminder::Model(ModelBody {
        model_ref: model_ref.to_string(),
        text,
    }))
}
