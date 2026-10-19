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

use std::mem::size_of;

use rquickjs::{Atom, Ctx, Error, Exception, Filter, Type, Value};
use serde_json::{Map, Number};

use super::MAX_SAFE_INTEGER;

/// Deepest array/object nesting accepted in either direction.
///
/// Both directions walk the value recursively, and the host-to-JS path then
/// re-enters `serde_json` and QuickJS `JSON.parse`, which are recursive too.
/// A Windows thread starts with a 1 MiB stack; QuickJS's own stack cap in this
/// runtime is a few hundred KiB. A conversion or parse frame is on the order of
/// a kilobyte, so 32 frames stay far under either limit. 20_000 frames overflow
/// the thread (`0xC00000FD`) before either engine can report an error.
pub(crate) const MAX_JSON_DEPTH: usize = 32;

pub(crate) const NESTING_LIMIT: &str = "nesting depth exceeded";

/// Largest array whose `Vec<Value>` fits in the output budget.
///
/// `serde_json::Value` is much wider than its JSON text. Checking this before
/// `Vec::with_capacity` keeps a sparse `length` of 1e8 from reserving gigabytes.
pub(crate) fn max_json_elements(output_limit: usize) -> usize {
    output_limit / size_of::<serde_json::Value>().max(1)
}

#[derive(Debug)]
pub(crate) struct Thrown {
    pub message: String,
    pub stack: Option<String>,
}

#[derive(Debug)]
pub(crate) enum BoundaryError {
    Illegal(String),
    Thrown(Thrown),
}

pub(crate) fn from_js(
    value: &Value<'_>,
    output_limit: usize,
) -> Result<serde_json::Value, BoundaryError> {
    let mut stack = Vec::new();
    from_js_inner(value, &mut stack, 0, output_limit)
}

fn from_js_inner(
    value: &Value<'_>,
    stack: &mut Vec<usize>,
    depth: usize,
    output_limit: usize,
) -> Result<serde_json::Value, BoundaryError> {
    if depth > MAX_JSON_DEPTH {
        return Err(BoundaryError::Illegal(NESTING_LIMIT.into()));
    }
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
        return array_from_js(value, stack, depth, output_limit);
    }
    if value.is_object() {
        return object_from_js(value, stack, depth, output_limit);
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
    depth: usize,
    output_limit: usize,
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
        let len = array_length(value)?;
        if len > max_json_elements(output_limit) as u64 {
            return Err(BoundaryError::Illegal("array exceeds output limit".into()));
        }
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        reject_symbol_keys(value)?;
        let object = array.as_object();
        for index in 0..len {
            let key = u32::try_from(index)
                .map_err(|_| BoundaryError::Illegal("array exceeds output limit".into()))?;
            let present = object
                .contains_key(key)
                .map_err(|error| script_error(value, error))?;
            if !present {
                return Err(BoundaryError::Illegal("sparse array".into()));
            }
        }
        let mut items = Vec::with_capacity(len);
        for index in 0..len {
            let element = array
                .get::<Value<'_>>(index)
                .map_err(|error| script_error(value, error))?;
            if element.is_undefined() {
                return Err(BoundaryError::Illegal("undefined".into()));
            }
            items.push(from_js_inner(&element, stack, depth + 1, output_limit)?);
        }
        Ok(serde_json::Value::Array(items))
    })();
    stack.pop();
    result
}

fn array_length(value: &Value<'_>) -> Result<u64, BoundaryError> {
    let object = value
        .as_object()
        .ok_or_else(|| BoundaryError::Illegal("sparse array".into()))?;
    let length = object
        .get::<_, Value<'_>>("length")
        .map_err(|error| script_error(value, error))?;
    if let Some(int) = length.as_int() {
        return u64::try_from(int).map_err(|_| BoundaryError::Illegal("array length".into()));
    }
    if let Some(number) = length.as_number()
        && number.is_finite()
        && number >= 0.0
        && number.fract() == 0.0
        && number <= u64::MAX as f64
    {
        return Ok(number as u64);
    }
    Err(BoundaryError::Illegal("array length".into()))
}

fn object_from_js(
    value: &Value<'_>,
    stack: &mut Vec<usize>,
    depth: usize,
    output_limit: usize,
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
        reject_symbol_keys(value)?;
        let mut map = Map::new();
        for property in object.props::<String, Value<'_>>() {
            let (key, child) = property.map_err(|error| script_error(value, error))?;
            if child.is_undefined() {
                return Err(BoundaryError::Illegal("undefined property".into()));
            }
            map.insert(key, from_js_inner(&child, stack, depth + 1, output_limit)?);
        }
        Ok(serde_json::Value::Object(map))
    })();
    stack.pop();
    result
}

fn reject_symbol_keys(value: &Value<'_>) -> Result<(), BoundaryError> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    match object.own_keys::<Atom<'_>>(Filter::new().symbol()).next() {
        None => Ok(()),
        Some(Ok(_)) => Err(BoundaryError::Illegal("symbol key".into())),
        Some(Err(error)) => Err(script_error(value, error)),
    }
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
        })
    }
}

pub(crate) fn take_exception(ctx: &Ctx<'_>) -> Thrown {
    describe(&ctx.catch())
}

pub(crate) fn describe(value: &Value<'_>) -> Thrown {
    if let Some(exception) = value.as_exception() {
        return Thrown {
            message: exception.message().unwrap_or_else(|| "exception".into()),
            stack: exception.stack(),
        };
    }
    let message = value.get::<String>().unwrap_or_else(|_| "exception".into());
    Thrown {
        message,
        stack: None,
    }
}

/// Reject host integers outside the safe range and values nested past
/// [`MAX_JSON_DEPTH`], then parse. The depth check returns before `to_string`
/// or `JSON.parse`, so neither recursive walk can overflow the thread stack.
pub(crate) fn to_js<'js>(ctx: &Ctx<'js>, value: &serde_json::Value) -> Result<Value<'js>, String> {
    check_host_value(value, 0)?;
    let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
    match ctx.json_parse(text) {
        Ok(parsed) => Ok(parsed),
        Err(error) => {
            let _ = ctx.catch();
            Err(format!("host result could not be injected: {error}"))
        }
    }
}

fn check_host_value(value: &serde_json::Value, depth: usize) -> Result<(), String> {
    if depth > MAX_JSON_DEPTH {
        return Err(NESTING_LIMIT.into());
    }
    match value {
        serde_json::Value::Number(number) => check_number(number),
        serde_json::Value::Array(items) => {
            for item in items {
                check_host_value(item, depth + 1)?;
            }
            Ok(())
        }
        serde_json::Value::Object(map) => {
            for child in map.values() {
                check_host_value(child, depth + 1)?;
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
