//! Numeric input budgets.
//!
//! A budget is a policy, not a shape: the generated input schema describes the
//! range in prose, and the tool clamps whatever arrives instead of rejecting the
//! whole call. The note travels back in the tool's own `message` field.

use serde_json::Value;

/// One numeric parameter after clamping.
#[derive(Debug, Clone, PartialEq)]
pub struct Clamped<T> {
    pub value: T,
    /// Set when the requested value fell outside the budget.
    pub note: Option<String>,
}

/// Read an integer parameter and clamp it into `min..=max`.
///
/// Missing or `null` yields `None` (the caller applies its default). A
/// non-numeric value keeps the usual parameter error.
pub fn clamp_int(
    input: &Value,
    name: &str,
    min: u64,
    max: u64,
    unit: &str,
) -> Result<Option<Clamped<u64>>, String> {
    let Some(raw) = input.get(name).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let Some(requested) = int_number(raw) else {
        return Err(format!("parameter '{name}' expected an integer"));
    };
    let clamped = requested.clamp(i128::from(min), i128::from(max)) as u64;
    let note = (i128::from(clamped) != requested).then(|| int_note(name, requested, clamped, unit));
    Ok(Some(Clamped {
        value: clamped,
        note,
    }))
}

/// Read a float parameter and clamp it into `min..=max`.
pub fn clamp_float(
    input: &Value,
    name: &str,
    min: f64,
    max: f64,
    unit: &str,
) -> Result<Option<Clamped<f64>>, String> {
    let Some(raw) = input.get(name).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let Some(requested) = raw.as_f64() else {
        return Err(format!("parameter '{name}' expected a number"));
    };
    let clamped = requested.clamp(min, max);
    let note = (clamped != requested).then(|| float_note(name, requested, clamped, unit));
    Ok(Some(Clamped {
        value: clamped,
        note,
    }))
}

fn int_number(value: &Value) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(i128::from))
}

fn int_note(name: &str, from: i128, to: u64, unit: &str) -> String {
    let verb = if i128::from(to) < from {
        "reduced"
    } else {
        "raised"
    };
    note(name, verb, from, to, unit)
}

fn float_note(name: &str, from: f64, to: f64, unit: &str) -> String {
    let verb = if to < from { "reduced" } else { "raised" };
    note(name, verb, from, to, unit)
}

fn note(
    name: &str,
    verb: &str,
    from: impl std::fmt::Display,
    to: impl std::fmt::Display,
    unit: &str,
) -> String {
    let unit = if unit.is_empty() {
        String::new()
    } else {
        format!(" {unit}")
    };
    format!("`{name}` was {verb} from {from} to {to}{unit}.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_and_null_yield_none() {
        assert_eq!(clamp_int(&json!({}), "limit", 1, 100, "hits"), Ok(None));
        assert_eq!(
            clamp_int(&json!({"limit": null}), "limit", 1, 100, "hits"),
            Ok(None)
        );
    }

    #[test]
    fn in_range_keeps_the_value_without_a_note() {
        let clamped = clamp_int(&json!({"limit": 40}), "limit", 1, 100, "hits")
            .unwrap()
            .unwrap();
        assert_eq!(clamped.value, 40);
        assert!(clamped.note.is_none());
    }

    #[test]
    fn above_the_budget_is_reduced_with_a_note() {
        let clamped = clamp_int(&json!({"limit": 500}), "limit", 1, 100, "hits")
            .unwrap()
            .unwrap();
        assert_eq!(clamped.value, 100);
        assert_eq!(
            clamped.note.as_deref(),
            Some("`limit` was reduced from 500 to 100 hits.")
        );
    }

    #[test]
    fn below_the_budget_is_raised_with_a_note() {
        let clamped = clamp_int(&json!({"limit": 0}), "limit", 1, 100, "hits")
            .unwrap()
            .unwrap();
        assert_eq!(clamped.value, 1);
        assert_eq!(
            clamped.note.as_deref(),
            Some("`limit` was raised from 0 to 1 hits.")
        );
    }

    #[test]
    fn negative_values_are_raised_instead_of_rejected() {
        let clamped = clamp_int(&json!({"limit": -5}), "limit", 1, 100, "hits")
            .unwrap()
            .unwrap();
        assert_eq!(clamped.value, 1);
        assert!(clamped.note.unwrap().contains("raised from -5"));
    }

    #[test]
    fn a_non_number_keeps_the_parameter_error() {
        let error = clamp_int(&json!({"limit": "many"}), "limit", 1, 100, "hits").unwrap_err();
        assert_eq!(error, "parameter 'limit' expected an integer");
    }

    #[test]
    fn floats_clamp_to_their_range() {
        let high = clamp_float(&json!({"min_score": 1.5}), "min_score", 0.0, 1.0, "")
            .unwrap()
            .unwrap();
        assert_eq!(high.value, 1.0);
        assert_eq!(
            high.note.as_deref(),
            Some("`min_score` was reduced from 1.5 to 1.")
        );

        let low = clamp_float(&json!({"min_score": -0.25}), "min_score", 0.0, 1.0, "")
            .unwrap()
            .unwrap();
        assert_eq!(low.value, 0.0);
        assert_eq!(
            low.note.as_deref(),
            Some("`min_score` was raised from -0.25 to 0.")
        );
    }

    #[test]
    fn float_accepts_integer_json_numbers() {
        let clamped = clamp_float(&json!({"min_score": 1}), "min_score", 0.0, 1.0, "")
            .unwrap()
            .unwrap();
        assert_eq!(clamped.value, 1.0);
        assert!(clamped.note.is_none());
    }
}
