//! Reproduction of weak-model Python indent / whitespace mistakes against edit.
//! These tests lock current matcher behavior so we can tell tool defects from
//! model-only failures. See the evaluation notes in each test name.

use tokio_util::sync::CancellationToken;

use super::feedback::report;
use super::matcher::{RejectReason, WhitespaceKind};
use super::planner::{BlockDecision, EditBlock, PlannedBatch, plan_edits};

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

fn plan_of(content: &str, edits: &[(&str, &str, bool)]) -> PlannedBatch {
    let blocks: Vec<EditBlock> = edits
        .iter()
        .enumerate()
        .map(|(i, (old, new, replace_all))| EditBlock {
            index: i + 1,
            old_string: (*old).into(),
            new_string: (*new).into(),
            replace_all: *replace_all,
        })
        .collect();
    plan_edits(content, &blocks, &CancellationToken::new()).unwrap()
}

fn accepted(batch: &PlannedBatch, index: usize) -> &super::planner::BlockPlan {
    match &batch.decisions[index] {
        BlockDecision::Accept(plan) => plan,
        BlockDecision::Reject(failure) => panic!("expected accept, got {failure:?}"),
    }
}

fn rejected(batch: &PlannedBatch, index: usize) -> &super::planner::BlockFailure {
    match &batch.decisions[index] {
        BlockDecision::Reject(failure) => failure,
        BlockDecision::Accept(plan) => panic!("expected reject, got {plan:?}"),
    }
}

fn reason_of(content: &str, old: &str, new: &str) -> Option<RejectReason> {
    match &plan_of(content, &[(old, new, false)]).decisions[0] {
        BlockDecision::Reject(failure) => Some(failure.reason.clone()),
        BlockDecision::Accept(_) => None,
    }
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
    let batch = plan_of(PY, &[(old, "CHANGED", false)]);
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::WhitespaceOnly {
                kind: WhitespaceKind::LeadingTrailing,
                ..
            }
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
    assert!(batch.edited.is_none());
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
    let batch = plan_of(PY, &[(old, "CHANGED", false)]);
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::WhitespaceOnly { .. }
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
    assert!(batch.edited.is_none());
}

/// Tabs vs spaces on an otherwise unique block.
#[test]
fn tabs_vs_spaces_is_whitespace_only_reject() {
    let content = "def foo():\n    return 1\n";
    let old = "def foo():\n\treturn 1";
    let batch = plan_of(content, &[(old, "def foo():\n    return 2", false)]);
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::WhitespaceOnly {
                kind: WhitespaceKind::InternalSpacing | WhitespaceKind::LeadingTrailing,
                ..
            }
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
    assert!(batch.edited.is_none());
}

/// Exact old_string, but new_string drops a Python indent level.
/// Tool applies it: matching succeeded, replacement is trusted.
#[test]
fn exact_old_wrong_new_indent_applies_and_breaks_python() {
    let old = "        if item.active:\n            result.append(item.value)";
    let new = "if item.active:\n    result.append(item.value * 2)";
    let batch = plan_of(PY, &[(old, new, false)]);
    let _edited = accepted(&batch, 0);
    let out = batch.edited.as_deref().unwrap();
    assert!(
        out.contains("\nif item.active:\n    result.append(item.value * 2)\n"),
        "{out}"
    );
    assert!(!out.contains("            result.append(item.value * 2)"));
}

/// Under-indented old_string is a *substring* of a deeper-indented line.
/// Exact match fires; leftover leading spaces + new_string decide the result.
#[test]
fn underindented_old_substring_matches_deeper_line() {
    let content = "def foo():\n        unique_token = 1\n";
    // 4 spaces, file has 8. "    unique_token = 1" is a suffix of the 8-space line.
    let old = "    unique_token = 1";
    let batch_keep = plan_of(content, &[(old, "    unique_token = 2", false)]);
    assert!(matches!(
        &batch_keep.decisions[0],
        BlockDecision::Accept(_)
    ));
    assert_eq!(
        batch_keep.edited.as_deref(),
        Some("def foo():\n        unique_token = 2\n")
    );

    let batch_drop = plan_of(content, &[(old, "unique_token = 2", false)]);
    assert!(matches!(
        &batch_drop.decisions[0],
        BlockDecision::Accept(_)
    ));
    assert_eq!(
        batch_drop.edited.as_deref(),
        Some("def foo():\n    unique_token = 2\n"),
        "leftover 4 spaces + unindented new_string dedents the line"
    );
}

/// Same token at two indent levels: under-indented old_string hits both
/// (shallower line fully, deeper line as substring) → multiple_exact.
#[test]
fn underindented_old_hits_two_indent_levels_as_multiple_exact() {
    let content = "\
def foo():
    token = 1
    if token:
        token = 1
";
    let batch = plan_of(content, &[("    token = 1", "    token = 2", false)]);
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::MultipleExact { .. }
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
    assert!(batch.edited.is_none());
}

/// Over-indented old_string is not a substring; unique trim match → whitespace_only.
#[test]
fn overindented_old_is_whitespace_only_not_substring() {
    let content = "def foo():\n    unique_token = 1\n";
    let old = "        unique_token = 1";
    let batch = plan_of(content, &[(old, "        unique_token = 2", false)]);
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::WhitespaceOnly { .. }
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
    assert!(batch.edited.is_none());
}

/// Model includes read's `     N: ` gutter. Rejected and not applied.
#[test]
fn read_line_prefix_with_python_indent_is_rejected() {
    let line = "            result.append(item.value)";
    let prefixed = crate::tool::format_file_line(5, line);
    let batch = plan_of(PY, &[(prefixed.trim_end(), "CHANGED", false)]);
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::ReadLinePrefix { .. }
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
    assert!(batch.edited.is_none());
}

/// Weak model copies the colon-space from the read gutter onto the source indent.
/// `     5:             result...` stripped of digits still has an extra space after `:`.
/// If they copy ` :            result` that is a different failure.
/// Here: they copy source but keep one extra leading space from the gutter.
#[test]
fn extra_space_from_read_gutter_is_whitespace_only() {
    let old = "             result.append(item.value)"; // 13 spaces; file has 12
    let batch = plan_of(
        PY,
        &[(old, "            result.append(item.value * 2)", false)],
    );
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::WhitespaceOnly { .. }
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
}

/// Trailing spaces after a unique token: exact substring still hits, leaving
/// the trailing spaces in place. Not diagnosed as whitespace_only.
#[test]
fn trailing_file_whitespace_kept_by_substring_exact() {
    let content = "def foo():\n    unique_token = 1   \n";
    let old = "    unique_token = 1";
    let batch = plan_of(content, &[(old, "    unique_token = 2", false)]);
    // Substring: "    unique_token = 1" matches before the trailing spaces.
    assert!(matches!(&batch.decisions[0], BlockDecision::Accept(_)));
    assert_eq!(
        batch.edited.as_deref(),
        Some("def foo():\n    unique_token = 2   \n")
    );
}

/// Blank line that is indented in the file (`    \n`) vs a bare newline in old.
#[test]
fn indented_blank_line_vs_bare_newline_is_whitespace_only() {
    let content = "def foo():\n    x = 1\n    \n    y = 2\n";
    let old = "    x = 1\n\n    y = 2";
    let batch = plan_of(content, &[(old, "    x = 1\n\n    y = 3", false)]);
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::WhitespaceOnly { .. }
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
}

/// A typo is not an exact match, even when the indent is also wrong.
#[test]
fn typo_plus_missing_indent_is_no_match() {
    let content = "\
def greet_user_alpha():
    print(\"hi\")
";
    let old = "\
def greet_user_alpa():
print(\"hi\")";
    let new = "\
def greet_user_beta():
print(\"hi\")";
    let batch = plan_of(content, &[(old, new, false)]);
    assert!(matches!(rejected(&batch, 0).reason, RejectReason::NoMatch));
    assert!(batch.edited.is_none());
}

/// The same typo with the file's indent still does not match.
#[test]
fn typo_with_correct_indent_is_no_match() {
    let content = "\
def greet_user_alpha():
    print(\"hi\")
";
    let old = "\
def greet_user_alpa():
    print(\"hi\")";
    let new = "\
def greet_user_beta():
    print(\"hi\")";
    let batch = plan_of(content, &[(old, new, false)]);
    assert!(matches!(rejected(&batch, 0).reason, RejectReason::NoMatch));
    assert!(batch.edited.is_none());
}

/// Content change with fully wrong indent: whitespace_only (trim of *old* vs file)
/// blocks the intended semantic edit. Weak models retry here.
#[test]
fn intended_content_change_blocked_when_old_indent_wrong() {
    let old = "\
def process(items):
result = []
for item in items:
if item.active:
result.append(item.value)
else:
result.append(0)
return result";
    let new = old.replace("item.value", "item.value * 2");
    let batch = plan_of(PY, &[(old, &new, false)]);
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::WhitespaceOnly { .. }
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
    assert!(batch.edited.is_none());
}

/// JSON-ish mistake: literal backslash-n instead of newlines.
#[test]
fn literal_backslash_n_does_not_match_newlines() {
    let old = "def process(items):\\n    result = []";
    let batch = plan_of(PY, &[(old, "x", false)]);
    assert!(
        matches!(
            rejected(&batch, 0).reason,
            RejectReason::NoMatch
        ),
        "{:?}",
        rejected(&batch, 0).reason
    );
}

/// Correct indent exact replace still works for a nested Python edit.
#[test]
fn correct_indent_exact_replace_succeeds() {
    let old = "            result.append(item.value)";
    let new = "            result.append(item.value * 2)";
    let batch = plan_of(PY, &[(old, new, false)]);
    assert!(matches!(&batch.decisions[0], BlockDecision::Accept(_)));
    assert!(
        batch
            .edited
            .as_deref()
            .unwrap()
            .contains("            result.append(item.value * 2)")
    );
}

/// Weak model copies only the unique identifier, which substring-matches
/// inside the indented line and leaves the file indent intact.
#[test]
fn identifier_only_old_keeps_surrounding_indent() {
    let batch = plan_of(PY, &[("item.value)", "item.value * 2)", false)]);
    assert!(matches!(&batch.decisions[0], BlockDecision::Accept(_)));
    assert!(
        batch
            .edited
            .as_deref()
            .unwrap()
            .contains("            result.append(item.value * 2)")
    );
}

#[test]
fn whitespace_only_reason_exposes_file_preview() {
    let reason = reason_of(
        "def foo():\n    unique_name_here = 1\n",
        "def foo():\nunique_name_here = 1",
        "x",
    )
    .unwrap();
    match reason {
        RejectReason::WhitespaceOnly { file_preview, .. } => {
            // Preview is the first line of the window, not the first indented mismatch.
            assert_eq!(file_preview, "def foo():");
        }
        other => panic!("expected whitespace_only, got {other:?}"),
    }
}

#[test]
fn whitespace_only_feedback_does_not_show_the_indented_line() {
    let planned = plan_of(
        "def foo():\n    unique_name_here = 1\n",
        &[("def foo():\nunique_name_here = 1", "x", false)],
    );
    let reported = report(&planned);
    let failed = reported.blocks.iter().find(|block| block.status == "failed").unwrap();
    let reason = failed.reason.as_ref().unwrap();
    assert_eq!(reason.code, "whitespace_only");
    assert!(!reason.message.contains("unique_name_here"), "{}", reason.message);
    let preview = reason.details.as_ref().unwrap()["excerpt"].as_str().unwrap();
    assert_eq!(preview, "def foo():");
    assert!(!preview.contains("unique_name_here"));
}

/// Two identical trimmed blocks: indent-only old_string is not unique, so
/// whitespace_only does not fire and the block is no_match.
#[test]
fn duplicate_trimmed_blocks_skip_whitespace_only() {
    let content = "\
class A:
    def foo():
        return 1

class B:
    def foo():
        return 1
";
    let old = "def foo():\nreturn 1";
    let batch = plan_of(content, &[(old, "CHANGED", false)]);
    assert!(
        matches!(rejected(&batch, 0).reason, RejectReason::NoMatch),
        "{:?}",
        rejected(&batch, 0).reason
    );
    assert!(batch.edited.is_none());
}
