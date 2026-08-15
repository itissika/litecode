use crate::types::LitecodeError;

/// L0 agent loop termination — mapped to `TurnEndReason` by L1 runtime.
#[derive(Debug)]
pub enum TurnOutcome {
    Completed { final_text: String },
    Cancelled { final_text: String },
    MaxSteps { final_text: String },
    Error(LitecodeError),
}

impl TurnOutcome {
    pub fn final_text(&self) -> Option<&str> {
        match self {
            TurnOutcome::Completed { final_text }
            | TurnOutcome::Cancelled { final_text }
            | TurnOutcome::MaxSteps { final_text } => Some(final_text.as_str()),
            TurnOutcome::Error(_) => None,
        }
    }
}
