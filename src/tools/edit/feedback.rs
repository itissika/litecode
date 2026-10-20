use serde::Serialize;
use serde_json::{Value, json};

use crate::tool::outcome::Diagnostic;

use super::matcher::RejectReason;
use super::planner::{BlockDecision, PlannedBatch};

#[derive(Debug, Clone, Serialize)]
pub(super) struct LineRange {
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BlockReason {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct EditBlockOut {
    pub index: usize,
    pub status: &'static str,
    pub ranges: Vec<LineRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<BlockReason>,
}

pub(super) struct EditReport {
    pub blocks: Vec<EditBlockOut>,
    pub applied: usize,
    pub failed: usize,
    pub diagnostics: Vec<Diagnostic>,
}

pub(super) fn report(planned: &PlannedBatch) -> EditReport {
    let mut blocks = Vec::with_capacity(planned.decisions.len());
    let mut diagnostics = Vec::new();
    let mut applied = 0usize;
    let mut failed = 0usize;
    for decision in &planned.decisions {
        match decision {
            BlockDecision::Accept(plan) => {
                applied += 1;
                blocks.push(EditBlockOut {
                    index: plan.index,
                    status: "applied",
                    ranges: plan
                        .replacements
                        .iter()
                        .map(|item| LineRange {
                            start_line: item.span.start_line,
                            end_line: item.span.end_line,
                        })
                        .collect(),
                    reason: None,
                });
            }
            BlockDecision::Reject(failure) => {
                failed += 1;
                let reason = reason_of(&failure.reason);
                let ranges = ranges_of(&failure.reason);
                diagnostics.push(
                    Diagnostic::warning(
                        "edit_block_failed",
                        format!(
                            "edit block {} failed: {}",
                            failure.index,
                            failure.reason.code()
                        ),
                    )
                    .with_details(json!({
                        "index": failure.index,
                        "reason": &reason,
                        "ranges": &ranges,
                    })),
                );
                blocks.push(EditBlockOut {
                    index: failure.index,
                    status: "failed",
                    ranges,
                    reason: Some(reason),
                });
            }
        }
    }
    EditReport {
        blocks,
        applied,
        failed,
        diagnostics,
    }
}

fn reason_of(reason: &RejectReason) -> BlockReason {
    BlockReason {
        code: reason.code().to_string(),
        message: reason.message(),
        details: reason.details(),
    }
}

fn ranges_of(reason: &RejectReason) -> Vec<LineRange> {
    reason
        .ranges()
        .into_iter()
        .map(|(start_line, end_line)| LineRange {
            start_line,
            end_line,
        })
        .collect()
}
