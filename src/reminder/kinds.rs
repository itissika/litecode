//! Typed reminder bodies. `text` is frozen when the row is written.

use serde::{Deserialize, Serialize};

/// Whether HumanView renders the row. The wire copies this as `hidden`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
}

/// Stable reminder discriminator. The log kind is `reminder/<name>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderKind {
    Env,
    Tasks,
    Background,
    PlanChanged,
    FilesChanged,
    BashExit,
    SubagentSettled,
    CustomToolSettled,
    StepBudget,
    /// One-shot attachment after a user message. Not restored after compaction.
    Mentions,
    /// Knowledge-base class, attached after a human message. Hidden.
    KnowledgeStatus,
}

impl ReminderKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Tasks => "tasks",
            Self::Background => "background",
            Self::PlanChanged => "plan_changed",
            Self::FilesChanged => "files_changed",
            Self::BashExit => "bash_exit",
            Self::SubagentSettled => "subagent_settled",
            Self::CustomToolSettled => "custom_tool_settled",
            Self::StepBudget => "step_budget",
            Self::Mentions => "mentions",
            Self::KnowledgeStatus => "knowledge_status",
        }
    }

    pub fn wire(self) -> &'static str {
        match self {
            Self::Env => "reminder/env",
            Self::Tasks => "reminder/tasks",
            Self::Background => "reminder/background",
            Self::PlanChanged => "reminder/plan_changed",
            Self::FilesChanged => "reminder/files_changed",
            Self::BashExit => "reminder/bash_exit",
            Self::SubagentSettled => "reminder/subagent_settled",
            Self::CustomToolSettled => "reminder/custom_tool_settled",
            Self::StepBudget => "reminder/step_budget",
            Self::Mentions => "reminder/mentions",
            Self::KnowledgeStatus => "reminder/knowledge_status",
        }
    }

    pub fn parse_wire(value: &str) -> Option<Self> {
        Some(match value {
            "reminder/env" => Self::Env,
            "reminder/tasks" => Self::Tasks,
            "reminder/background" => Self::Background,
            "reminder/plan_changed" => Self::PlanChanged,
            "reminder/files_changed" => Self::FilesChanged,
            "reminder/bash_exit" => Self::BashExit,
            "reminder/subagent_settled" => Self::SubagentSettled,
            "reminder/custom_tool_settled" => Self::CustomToolSettled,
            "reminder/step_budget" => Self::StepBudget,
            "reminder/mentions" => Self::Mentions,
            "reminder/knowledge_status" => Self::KnowledgeStatus,
            _ => return None,
        })
    }

    pub fn is_wire(value: &str) -> bool {
        value.starts_with("reminder/")
    }

    pub fn visibility(self) -> Visibility {
        match self {
            Self::BashExit | Self::SubagentSettled | Self::CustomToolSettled => Visibility::Visible,
            Self::Env
            | Self::Tasks
            | Self::Background
            | Self::PlanChanged
            | Self::FilesChanged
            | Self::StepBudget
            | Self::Mentions
            | Self::KnowledgeStatus => Visibility::Hidden,
        }
    }

    pub fn hidden(self) -> bool {
        self.visibility() == Visibility::Hidden
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvBody {
    pub cwd: String,
    pub os: String,
    pub date: String,
    pub timezone: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoSnap {
    pub id: String,
    pub content: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanPointer {
    pub relative_path: String,
    pub slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TasksBody {
    pub todos: Vec<TodoSnap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_plan: Option<PlanPointer>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningBash {
    pub job_id: String,
    pub command: String,
    pub output_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildCountsBody {
    pub running: usize,
    pub idle: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundBody {
    pub running_bash: Vec<RunningBash>,
    pub children: ChildCountsBody,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanChangedBody {
    pub relative_path: String,
    pub revision: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesChangedBody {
    pub paths: Vec<String>,
    pub more: usize,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BashExitEntry {
    pub job_id: String,
    pub command: String,
    pub exit_code: i32,
    pub killed: bool,
    pub output_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BashExitBody {
    pub exits: Vec<BashExitEntry>,
    pub running: Vec<RunningBash>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettledChild {
    pub child_session_id: String,
    pub turn_id: String,
    pub agent: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentSettledBody {
    pub settled: Vec<SettledChild>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomToolSettledEntry {
    pub job_id: String,
    pub call_id: String,
    pub tool_name: String,
    /// `ok` | `error` | `cancelled`
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomToolSettledBody {
    pub settled: Vec<CustomToolSettledEntry>,
    pub text: String,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepBudgetBody {
    pub step: u64,
    pub max_steps: u64,
    pub turn_id: String,
    pub text: String,
}

/// One citation expanded into the frozen mentions attachment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionRef {
    pub kind: String,
    pub key: String,
    /// Hash of the stable snapshot. Absent on rows written before dedupe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionsBody {
    pub refs: Vec<MentionRef>,
    pub text: String,
}

/// Knowledge-base class frozen at the human message. `class` is the dedupe key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeStatusBody {
    pub class: String,
    pub text: String,
}

/// One durable reminder. The `kind` tag matches [`ReminderKind::name`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reminder {
    Env(EnvBody),
    Tasks(TasksBody),
    Background(BackgroundBody),
    PlanChanged(PlanChangedBody),
    FilesChanged(FilesChangedBody),
    BashExit(BashExitBody),
    SubagentSettled(SubagentSettledBody),
    CustomToolSettled(CustomToolSettledBody),
    StepBudget(StepBudgetBody),
    Mentions(MentionsBody),
    KnowledgeStatus(KnowledgeStatusBody),
}

impl Reminder {
    pub fn kind(&self) -> ReminderKind {
        match self {
            Self::Env(_) => ReminderKind::Env,
            Self::Tasks(_) => ReminderKind::Tasks,
            Self::Background(_) => ReminderKind::Background,
            Self::PlanChanged(_) => ReminderKind::PlanChanged,
            Self::FilesChanged(_) => ReminderKind::FilesChanged,
            Self::BashExit(_) => ReminderKind::BashExit,
            Self::SubagentSettled(_) => ReminderKind::SubagentSettled,
            Self::CustomToolSettled(_) => ReminderKind::CustomToolSettled,
            Self::StepBudget(_) => ReminderKind::StepBudget,
            Self::Mentions(_) => ReminderKind::Mentions,
            Self::KnowledgeStatus(_) => ReminderKind::KnowledgeStatus,
        }
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Env(body) => &body.text,
            Self::Tasks(body) => &body.text,
            Self::Background(body) => &body.text,
            Self::PlanChanged(body) => &body.text,
            Self::FilesChanged(body) => &body.text,
            Self::BashExit(body) => &body.text,
            Self::SubagentSettled(body) => &body.text,
            Self::CustomToolSettled(body) => &body.text,
            Self::StepBudget(body) => &body.text,
            Self::Mentions(body) => &body.text,
            Self::KnowledgeStatus(body) => &body.text,
        }
    }

    /// Snapshot identity for Diff kinds. `text` is not part of the comparison.
    pub fn same_snapshot(&self, other: &Reminder) -> bool {
        match (self, other) {
            (Self::Env(a), Self::Env(b)) => {
                a.cwd == b.cwd && a.os == b.os && a.date == b.date && a.timezone == b.timezone
            }
            (Self::PlanChanged(a), Self::PlanChanged(b)) => {
                a.relative_path == b.relative_path && a.revision == b.revision
            }
            (Self::KnowledgeStatus(a), Self::KnowledgeStatus(b)) => a.class == b.class,
            _ => false,
        }
    }
}
