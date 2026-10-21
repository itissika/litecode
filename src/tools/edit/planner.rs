//! Single-replacement planning for `edit` / `apply_edits`.

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::matcher::{self, RejectReason, ResolveOutcome};

pub(crate) const CANCELLED_MESSAGE: &str = "edit cancelled before write; file was not modified";
pub(crate) const EMPTY_FILE_MESSAGE: &str =
    "The file is empty. old_string cannot match an empty file.";

#[derive(Clone, Debug)]
pub(crate) struct EditSpec {
    pub old_string: String,
    pub new_string: String,
}

/// Parse `{ old_string, new_string }` from a bag. `allowed_extra` keys (e.g. `path`
/// on an apply_edits item) are ignored; retired knobs are rejected by name.
pub(crate) fn parse_spec(
    input: &Value,
    prefix: &str,
    allowed_extra: &[&str],
) -> Result<EditSpec, String> {
    let Some(obj) = input.as_object() else {
        return Err(crate::tool::expected_type(
            if prefix.is_empty() { "input" } else { prefix },
            "object",
            input,
        ));
    };
    for key in obj.keys() {
        if matches!(key.as_str(), "old_string" | "new_string")
            || allowed_extra.iter().any(|allowed| *allowed == key.as_str())
        {
            continue;
        }
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        return Err(crate::tool::schema_validate::unknown_parameter(&path));
    }

    let old = required_string(input, prefix, "old_string")?;
    let new = required_string(input, prefix, "new_string")?;
    Ok(EditSpec {
        old_string: old,
        new_string: new,
    })
}

fn required_string(input: &Value, prefix: &str, field: &str) -> Result<String, String> {
    let path = if prefix.is_empty() {
        field.to_string()
    } else {
        format!("{prefix}.{field}")
    };
    match input.get(field) {
        None => Err(crate::tool::missing_parameter(&path)),
        Some(value) => crate::tool::require_string_value(value, &path).map(str::to_string),
    }
}

/// Plan one unique replacement against `content`. Returns edited text on success.
pub(crate) fn plan_one(
    content: &str,
    spec: &EditSpec,
    cancel: &CancellationToken,
) -> Result<String, PlanFail> {
    if cancel.is_cancelled() {
        return Err(PlanFail::Cancelled);
    }
    if content.is_empty() {
        return Err(PlanFail::EmptyFile);
    }
    match matcher::resolve_one(content, &spec.old_string, &spec.new_string) {
        ResolveOutcome::Apply(edited) => {
            if cancel.is_cancelled() {
                return Err(PlanFail::Cancelled);
            }
            Ok(edited)
        }
        ResolveOutcome::Reject(reason) => Err(PlanFail::Reject(reason)),
    }
}

#[derive(Debug)]
pub(crate) enum PlanFail {
    Cancelled,
    EmptyFile,
    Reject(RejectReason),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    #[test]
    fn parse_rejects_replace_all_and_expected_hash() {
        let err = parse_spec(
            &serde_json::json!({
                "old_string": "a",
                "new_string": "b",
                "replace_all": true
            }),
            "",
            &[],
        )
        .unwrap_err();
        assert!(err.contains("replace_all"), "{err}");

        let err = parse_spec(
            &serde_json::json!({
                "old_string": "a",
                "new_string": "b",
                "expected_hash": "abc"
            }),
            "",
            &[],
        )
        .unwrap_err();
        assert!(err.contains("expected_hash"), "{err}");

        let err = parse_spec(
            &serde_json::json!({
                "path": "a.rs",
                "edits": [{"old_string": "a", "new_string": "b"}]
            }),
            "",
            &["path"],
        )
        .unwrap_err();
        assert!(err.contains("edits"), "{err}");
    }

    #[test]
    fn plan_one_unique_and_multiple() {
        let cancel = CancellationToken::new();
        let edited = plan_one(
            "fn start() {}\n",
            &EditSpec {
                old_string: "fn start() {}".into(),
                new_string: "fn main() {}".into(),
            },
            &cancel,
        )
        .unwrap();
        assert_eq!(edited, "fn main() {}\n");

        let fail = plan_one(
            "foo\nbar\nfoo\n",
            &EditSpec {
                old_string: "foo".into(),
                new_string: "FOO".into(),
            },
            &cancel,
        )
        .unwrap_err();
        assert!(matches!(
            fail,
            PlanFail::Reject(RejectReason::MultipleExact { .. })
        ));
    }

    #[test]
    fn plan_one_empty_new_deletes() {
        let edited = plan_one(
            "keep\nremove-me\n",
            &EditSpec {
                old_string: "remove-me\n".into(),
                new_string: String::new(),
            },
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(edited, "keep\n");
    }
}
