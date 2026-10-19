//! Lossless JSON boundary between JavaScript and the host.
//!
//! JavaScript to host (call arguments and the program result) accepts only
//! `null`, booleans, finite numbers, strings, dense arrays, and plain objects.
//! Anything else is [`BoundaryError::Illegal`].
//!
//! Host to JavaScript walks JSON and rejects integers whose magnitude exceeds
//! [`MAX_SAFE_INTEGER`](super::MAX_SAFE_INTEGER). Those integers are not all
//! exact in an IEEE-754 number, so injecting them would drop bits. The call
//! fails with a type error and the number is not delivered. `2^53 - 1` is
//! accepted; `2^53` is rejected. Non-integer JSON numbers are injected as
//! numbers because `serde_json` already stores them as `f64`.

use rquickjs::{Ctx, Error, Exception, Type, Value};
use serde_json::{Map, Number};

use super::MAX_SAFE_INTEGER;

#[derive(Debug)]
pub(crate) struct Thrown {
    pub message: String,
    pub stack: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug)]
pub(crate) enum BoundaryError {
    Illegal(String),
    Thrown(Thrown),
}

pub(crate) fn from_js(value: &Value<'_>) -> Result<serde_json::Value, BoundaryError> {
    let mut stack = Vec::new();
    from_js_inner(value, &mut stack)
}

fn from_js_inner(
    value: &Value<'_>,
    stack: &mut Vec<usize>,
) -> Result<serde_json::Value, BoundaryError> {
    if value.is_null() {
        return Ok(serde_json::Value::Null);
    }
    if value.is_undefined() {
        return Err(BoundaryError::Illegal("undefined".into()));
    }
    if value.is_bool() {
        return Ok(serde_json::Value::Bool(value.as_bool().unwrap_or(false)));
    }
    if let Some(number) = value.as_number() {
        return number_from_f64(number);
    }
    if value.is_string() {
        return Ok(serde_json::Value::String(rust_string(value)?));
    }
    if value.is_big_int() {
        return Err(BoundaryError::Illegal("bigint".into()));
    }
    if value.is_symbol() {
        return Err(BoundaryError::Illegal("symbol".into()));
    }
    if value.is_function() {
        return Err(BoundaryError::Illegal("function".into()));
    }
    if value.is_array() {
        return array_from_js(value, stack);
    }
    if value.is_object() {
        return object_from_js(value, stack);
    }
    Err(BoundaryError::Illegal(format!(
        "unsupported value {}",
        value.type_of().as_str()
    )))
}

fn number_from_f64(number: f64) -> Result<serde_json::Value, BoundaryError> {
    if !number.is_finite() {
        return Err(BoundaryError::Illegal("non-finite number".into()));
    }
    let max = MAX_SAFE_INTEGER as f64;
    if number.fract() == 0.0 && number.abs() <= max {
        let integer = number as i64;
        return Ok(serde_json::Value::Number(integer.into()));
    }
    Number::from_f64(number)
        .map(serde_json::Value::Number)
        .ok_or_else(|| BoundaryError::Illegal("non-finite number".into()))
}

fn array_from_js(
    value: &Value<'_>,
    stack: &mut Vec<usize>,
) -> Result<serde_json::Value, BoundaryError> {
    let array = value
        .as_array()
        .ok_or_else(|| BoundaryError::Illegal("sparse array".into()))?;
    let ptr = object_ptr(value);
    if stack.contains(&ptr) {
        return Err(BoundaryError::Illegal("circular reference".into()));
    }
    stack.push(ptr);
    let result = (|| {
        let len = array.len();
        let mut items = Vec::with_capacity(len);
        for index in 0..len {
            let present = array
                .as_object()
                .contains_key(index.to_string())
                .map_err(|error| script_error(value, error))?;
            if !present {
                return Err(BoundaryError::Illegal("sparse array".into()));
            }
            let element = array
                .get::<Value<'_>>(index)
                .map_err(|error| script_error(value, error))?;
            if element.is_undefined() {
                return Err(BoundaryError::Illegal("undefined".into()));
            }
            items.push(from_js_inner(&element, stack)?);
        }
        Ok(serde_json::Value::Array(items))
    })();
    stack.pop();
    result
}

fn object_from_js(
    value: &Value<'_>,
    stack: &mut Vec<usize>,
) -> Result<serde_json::Value, BoundaryError> {
    if !is_plain_object(value) {
        let name = constructor_name(value);
        return Err(BoundaryError::Illegal(format!(
            "unsupported object: {name}"
        )));
    }
    let object = value
        .as_object()
        .ok_or_else(|| BoundaryError::Illegal("unsupported object".into()))?;
    let ptr = object_ptr(value);
    if stack.contains(&ptr) {
        return Err(BoundaryError::Illegal("circular reference".into()));
    }
    stack.push(ptr);
    let result = (|| {
        let mut map = Map::new();
        for property in object.props::<String, Value<'_>>() {
            let (key, child) = property.map_err(|error| script_error(value, error))?;
            if child.is_undefined() {
                return Err(BoundaryError::Illegal("undefined property".into()));
            }
            map.insert(key, from_js_inner(&child, stack)?);
        }
        Ok(serde_json::Value::Object(map))
    })();
    stack.pop();
    result
}

fn is_plain_object(value: &Value<'_>) -> bool {
    if !value.is_object()
        || value.is_array()
        || value.is_function()
        || value.type_of() != Type::Object
    {
        return false;
    }
    let class_id = unsafe { rquickjs::qjs::JS_GetClassID(value.as_raw()) };
    class_id == 1
}

fn constructor_name(value: &Value<'_>) -> String {
    value
        .as_object()
        .and_then(|object| object.get::<_, rquickjs::Object<'_>>("constructor").ok())
        .and_then(|ctor| ctor.get::<_, String>("name").ok())
        .unwrap_or_else(|| "Object".into())
}

fn object_ptr(value: &Value<'_>) -> usize {
    unsafe { rquickjs::qjs::JS_VALUE_GET_PTR(value.as_raw()) as usize }
}

fn rust_string(value: &Value<'_>) -> Result<String, BoundaryError> {
    value
        .get::<String>()
        .map_err(|error| script_error(value, error))
}

fn script_error(value: &Value<'_>, error: Error) -> BoundaryError {
    if matches!(error, Error::Exception) {
        BoundaryError::Thrown(take_exception(value.ctx()))
    } else {
        BoundaryError::Thrown(Thrown {
            message: error.to_string(),
            stack: None,
            name: None,
        })
    }
}

pub(crate) fn take_exception(ctx: &Ctx<'_>) -> Thrown {
    let value = ctx.catch();
    let name = value
        .as_object()
        .and_then(|object| object.get::<_, String>("name").ok());
    if let Some(exception) = value.as_exception() {
        return Thrown {
            message: exception.message().unwrap_or_else(|| "exception".into()),
            stack: exception.stack(),
            name,
        };
    }
    let message = value.get::<String>().unwrap_or_else(|_| "exception".into());
    Thrown {
        message,
        stack: None,
        name,
    }
}

/// Reject host integers that are outside the safe integer range, then parse.
pub(crate) fn to_js<'js>(ctx: &Ctx<'js>, value: &serde_json::Value) -> Result<Value<'js>, String> {
    ensure_safe_integers(value)?;
    let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
    ctx.json_parse(text)
        .map_err(|error| format!("host result could not be injected: {error}"))
}

pub(crate) fn ensure_safe_integers(value: &serde_json::Value) -> Result<(), String> {
    match value {
        serde_json::Value::Number(number) => check_number(number),
        serde_json::Value::Array(items) => {
            for item in items {
                ensure_safe_integers(item)?;
            }
            Ok(())
        }
        serde_json::Value::Object(map) => {
            for child in map.values() {
                ensure_safe_integers(child)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn check_number(number: &Number) -> Result<(), String> {
    if let Some(integer) = number.as_i64() {
        if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&integer) {
            return Err(unsafe_integer());
        }
        return Ok(());
    }
    if let Some(integer) = number.as_u64() {
        if integer > MAX_SAFE_INTEGER as u64 {
            return Err(unsafe_integer());
        }
        return Ok(());
    }
    if let Some(float) = number.as_f64() {
        if !float.is_finite() {
            return Err("non-finite number".into());
        }
        if float.fract() == 0.0 && float.abs() > MAX_SAFE_INTEGER as f64 {
            return Err(unsafe_integer());
        }
        return Ok(());
    }
    Err(unsafe_integer())
}

fn unsafe_integer() -> String {
    "integer magnitude exceeds 2^53-1".into()
}

pub(crate) fn throw_type(ctx: &Ctx<'_>, message: &str) -> Error {
    Exception::throw_type(ctx, message)
}

pub(crate) fn throw_message(ctx: &Ctx<'_>, message: &str) -> Error {
    Exception::throw_message(ctx, message)
}

pub(crate) fn uncatchable(ctx: &Ctx<'_>, message: &str) -> Error {
    match Exception::from_message(ctx.clone(), message) {
        Ok(exception) => {
            unsafe {
                rquickjs::qjs::JS_SetUncatchableError(
                    ctx.as_raw().as_ptr(),
                    exception.as_value().as_raw(),
                );
            }
            exception.throw()
        }
        Err(error) => error,
    }
}
