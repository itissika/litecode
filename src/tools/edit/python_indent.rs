//! Reproduction of weak-model Python indent / whitespace mistakes against edit.
//! These tests lock current matcher behavior so we can tell tool defects from
//! model-only failures. See the evaluation notes in each test name.

use tokio_util::sync::CancellationToken;

use super::matcher::{RejectReason, ResolveOutcome, WhitespaceKind, resolve_one};
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

fn assert_apply(content: &str, old: &str, new: &str) -> String {
    plan_of(content, old, new).unwrap_or_else(|e| panic!("expected apply, got {e:?}"))
}

/// Typical weak-model copy: every line left-aligned. Content otherwise exact.
#[test]
fn fully_dedented_old_is_whitespace_only_reject() {
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
        matches!(
            reason,
            RejectReason::WhitespaceOnly {
                kind: WhitespaceKind::LeadingTrailing,
                ..
            }
        ),
        "{reason:?}"
    );
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

/// Tabs vs spaces on an otherwise unique block.
#[test]
fn tabs_vs_spaces_is_whitespace_only_reject() {
    let content = "def foo():\n    return 1\n";
    let old = "def foo():\n\treturn 1";
    let reason = assert_reject(content, old, "def foo():\n    return 2");
    assert!(
        matches!(
            reason,
            RejectReason::WhitespaceOnly {
                kind: WhitespaceKind::InternalSpacing | WhitespaceKind::LeadingTrailing,
                ..
            }
        ),
        "{reason:?}"
    );
}

/// Exact old_string, but new_string drops a Python indent level.
#[test]
fn exact_old_wrong_new_indent_applies_and_breaks_python() {
    let old = "        if item.active:\n            result.append(item.value)";
    let new = "if item.active:\n    result.append(item.value * 2)";
    let out = assert_apply(PY, old, new);
    assert!(
        out.contains("\nif item.active:\n    result.append(item.value * 2)\n"),
        "{out}"
    );
}

/// Under-indented old_string is a *substring* of a deeper-indented line.
#[test]
fn underindented_old_substring_matches_deeper_line() {
    let content = "def foo():\n        unique_token = 1\n";
    let out = assert_apply(content, "    unique_token = 1", "    unique_token = 2");
    assert!(out.contains("unique_token = 2"), "{out}");
}

#[test]
fn exact_match_still_applies() {
    let old = "        if item.active:\n            result.append(item.value)";
    let new = "        if item.active:\n            result.append(item.value * 2)";
    let out = assert_apply(PY, old, new);
    assert!(out.contains("item.value * 2"), "{out}");
}

#[test]
fn multiple_exact_rejects_without_replace_all() {
    let content = "x = 1\nx = 2\n";
    let reason = assert_reject(content, "x = ", "y = ");
    assert!(matches!(reason, RejectReason::MultipleExact { .. }), "{reason:?}");
    assert!(!reason.message().contains("replace_all"), "{}", reason.message());
}

#[test]
fn resolve_one_smoke() {
    match resolve_one("a\nb\n", "a", "A") {
        ResolveOutcome::Apply(s) => assert_eq!(s, "A\nb\n"),
        other => panic!("{other:?}"),
    }
}
