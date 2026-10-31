//! Reproduction of weak-model Python indent / whitespace mistakes against edit.
//! These tests lock current matcher behavior so we can tell tool defects from
//! model-only failures. See the evaluation notes in each test name.

use tokio_util::sync::CancellationToken;

use super::matcher::RejectReason;
use super::planner::{EditSpec, PlanFail, plan_one};

const PY: &str = "\
def process(items):
    result = []
    for item in items:
        if item.active:
            result.append(item.value)
        else:
            result.append(0)
    return result
";

fn plan_of(content: &str, old: &str, new: &str) -> Result<String, PlanFail> {
    plan_one(
        content,
        &EditSpec {
            old_string: old.into(),
            new_string: new.into(),
        },
        &CancellationToken::new(),
    )
}

fn assert_reject(content: &str, old: &str, new: &str) -> RejectReason {
    match plan_of(content, old, new) {
        Err(PlanFail::Reject(reason)) => reason,
        other => panic!("expected reject, got {other:?}"),
    }
}

/// 2-space indent vs file's 4-space. Content otherwise exact.
#[test]
fn two_space_vs_four_space_is_whitespace_only_reject() {
    let old = "\
def process(items):
  result = []
  for item in items:
    if item.active:
      result.append(item.value)
    else:
      result.append(0)
  return result";
    let reason = assert_reject(PY, old, "CHANGED");
    assert!(
        matches!(reason, RejectReason::WhitespaceOnly { .. }),
        "{reason:?}"
    );
}
