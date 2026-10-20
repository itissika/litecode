//! Lossless JSON boundary between JavaScript and the host.
//!
//! JavaScript to host (call arguments and the program result) accepts only
//! `null`, booleans, finite numbers, strings, dense arrays, and plain objects.
//! Anything else is [`BoundaryError::Illegal`].
//!
//! Both directions walk the value with an explicit heap stack. Nesting depth is
//! not capped: a deep directory tree or a long dense array is a real result.
//! The program output byte budget is the hard size limit, applied to the
//! encoded JSON. A sparse array is rejected from its populated keys before any
//! `Vec` is reserved for its length.
//!
//! Host to JavaScript rejects integers whose magnitude exceeds
//! [`MAX_SAFE_INTEGER`](super::MAX_SAFE_INTEGER). Those integers are not all
//! exact in an IEEE-754 number, so injecting them would drop bits. The call
//! fails with a type error and the number is not delivered. `2^53 - 1` is
//! accepted; `2^53` is rejected. Non-integer JSON numbers are injected as
//! numbers because `serde_json` already stores them as `f64`.

use std::collections::BTreeMap;

use rquickjs::{Array, Atom, Ctx, Error, Exception, Filter, IntoJs, Object, Type, Value};
use serde_json::{Map, Number};

use super::MAX_SAFE_INTEGER;
use super::json::Json;

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

enum Frame<'js> {
    Array {
        items: Vec<Json>,
        index: usize,
        len: usize,
        array: Array<'js>,
    },
    Object {
        map: BTreeMap<String, Json>,
        keys: Vec<String>,
        index: usize,
        object: Object<'js>,
    },
}

enum Walk<'js> {
    Down(Value<'js>),
    Up(Json),
}

fn ready<'js>(value: serde_json::Value) -> Walk<'js> {
    Walk::Up(Json::new(value))
}

pub(crate) fn from_js(value: &Value<'_>) -> Result<Json, BoundaryError> {
    let mut cycles = Vec::new();
    let mut frames: Vec<Frame<'_>> = Vec::new();
    let mut focus = value.clone();
    loop {
        match open(&focus, &mut cycles, &mut frames)? {
            Walk::Down(next) => focus = next,
            Walk::Up(mut json) => loop {
                if frames.is_empty() {
                    return Ok(json);
                }
                match finish(&mut frames, &mut cycles, json)? {
                    Walk::Down(next) => {
                        focus = next;
                        break;
                    }
                    Walk::Up(parent) => json = parent,
                }
            },
        }
    }
}

fn open<'js>(
    value: &Value<'js>,
    cycles: &mut Vec<usize>,
    frames: &mut Vec<Frame<'js>>,
) -> Result<Walk<'js>, BoundaryError> {
    if value.is_null() {
        return Ok(ready(serde_json::Value::Null));
    }
    if value.is_undefined() {
        return Err(undefined_at(frames.as_slice()));
    }
    if value.is_bool() {
        return Ok(ready(serde_json::Value::Bool(
            value.as_bool().unwrap_or(false),
        )));
    }
    if let Some(number) = value.as_number() {
        return number_from_f64(number).map(ready);
    }
    if value.is_string() {
        return Ok(ready(serde_json::Value::String(rust_string(value)?)));
    }
    if value.is_big_int() {
        return Err(illegal("bigint"));
    }
    if value.is_symbol() {
        return Err(illegal("symbol"));
    }
    if value.is_function() {
        return Err(illegal("function"));
    }
    if value.is_array() {
        return open_array(value, cycles, frames);
    }
    if value.is_object() {
        return open_object(value, cycles, frames);
    }
    Err(illegal(&format!(
        "unsupported value {}",
        value.type_of().as_str()
    )))
}

fn open_array<'js>(
    value: &Value<'js>,
    cycles: &mut Vec<usize>,
    frames: &mut Vec<Frame<'js>>,
) -> Result<Walk<'js>, BoundaryError> {
    let array = value
        .as_array()
        .ok_or_else(|| illegal("sparse array"))?
        .clone();
    let ptr = object_ptr(value);
    if cycles.contains(&ptr) {
        return Err(illegal("circular reference"));
    }
    reject_symbol_keys(value)?;
    let len = array_length(value)?;
    if len > u32::MAX as u64 {
        return Err(illegal("array length"));
    }
    // Populated keys decide sparseness. A huge `length` with few elements
    // returns here, before `try_reserve(len)`.
    ensure_dense(value, len)?;
    let len = usize::try_from(len).map_err(|_| illegal("array length"))?;
    if len == 0 {
        return Ok(ready(serde_json::Value::Array(Vec::new())));
    }
    let mut items = Vec::new();
    items
        .try_reserve_exact(len)
        .map_err(|_| illegal("array length"))?;
    frames.push(Frame::Array {
        items,
        index: 0,
        len,
        array,
    });
    cycles.push(ptr);
    Ok(Walk::Down(next_child(frames)?))
}

fn open_object<'js>(
    value: &Value<'js>,
    cycles: &mut Vec<usize>,
    frames: &mut Vec<Frame<'js>>,
) -> Result<Walk<'js>, BoundaryError> {
    if !is_plain_object(value) {
        let name = constructor_name(value);
        return Err(illegal(&format!("unsupported object: {name}")));
    }
    let object = value
        .as_object()
        .ok_or_else(|| illegal("unsupported object"))?
        .clone();
    let ptr = object_ptr(value);
    if cycles.contains(&ptr) {
        return Err(illegal("circular reference"));
    }
    reject_symbol_keys(value)?;
    let mut keys = Vec::new();
    for key in object.keys::<String>() {
        keys.push(key.map_err(|error| script_error(value, error))?);
    }
    if keys.is_empty() {
        return Ok(ready(serde_json::Value::Object(Map::new())));
    }
    frames.push(Frame::Object {
        map: BTreeMap::new(),
        keys,
        index: 0,
        object,
    });
    cycles.push(ptr);
    Ok(Walk::Down(next_child(frames)?))
}

fn next_child<'js>(frames: &[Frame<'js>]) -> Result<Value<'js>, BoundaryError> {
    match frames.last() {
        Some(Frame::Array { array, index, .. }) => {
            let element = array
                .get::<Value<'_>>(*index)
                .map_err(|error| script_error(array.as_value(), error))?;
            if element.is_undefined() {
                return Err(undefined_at(frames));
            }
            Ok(element)
        }
        Some(Frame::Object {
            object,
            keys,
            index,
            ..
        }) => {
            let child = object
                .get::<_, Value<'_>>(&keys[*index])
                .map_err(|error| script_error(object.as_value(), error))?;
            if child.is_undefined() {
                return Err(undefined_at(frames));
            }
            Ok(child)
        }
        None => Err(illegal("unsupported value")),
    }
}

/// JSON path of the value `frames` is currently reading. Diagnostics only.
///
/// Keys that are not plain identifiers use bracket notation so arbitrary keys
/// stay unambiguous (`$["a-b"]`), and string keys are JSON-escaped.
fn path_at(frames: &[Frame<'_>]) -> String {
    let mut path = String::from("$");
    for frame in frames {
        match frame {
            Frame::Array { index, .. } => {
                path.push('[');
                path.push_str(&index.to_string());
                path.push(']');
            }
            Frame::Object { keys, index, .. } => {
                let key = keys[*index].as_str();
                if is_identifier(key) {
                    path.push('.');
                    path.push_str(key);
                } else {
                    match serde_json::to_string(key) {
                        Ok(quoted) => {
                            path.push('[');
                            path.push_str(&quoted);
                            path.push(']');
                        }
                        Err(_) => path.push_str("[?]"),
                    }
                }
            }
        }
    }
    path
}

fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn undefined_at(frames: &[Frame<'_>]) -> BoundaryError {
    illegal(&format!(
        "undefined at {}: JSON has no undefined; use null or omit it",
        path_at(frames)
    ))
}

fn finish<'js>(
    frames: &mut Vec<Frame<'js>>,
    cycles: &mut Vec<usize>,
    json: Json,
) -> Result<Walk<'js>, BoundaryError> {
    let complete = match frames.last_mut() {
        Some(Frame::Array {
            items, index, len, ..
        }) => {
            items.push(json);
            *index += 1;
            *index == *len
        }
        Some(Frame::Object {
            map, keys, index, ..
        }) => {
            let key = keys[*index].clone();
            map.insert(key, json);
            *index += 1;
            *index == keys.len()
        }
        None => return Err(illegal("unsupported value")),
    };
    if !complete {
        return Ok(Walk::Down(next_child(frames)?));
    }
    cycles.pop();
    let json = match frames.pop() {
        Some(Frame::Array { items, .. }) => Json::from_array(items),
        Some(Frame::Object { map, .. }) => Json::from_object(map),
        None => return Err(illegal("unsupported value")),
    };
    Ok(Walk::Up(json))
}

/// Reject a sparse array before reserving `length` slots.
///
/// Own enumerable keys are the populated size. `new Array(1e8)` and
/// `array.length = 1e8` have almost none, so this returns without walking
/// `length` or allocating a result buffer. A fast array can also list a hole
/// in that key set; `contains_key` is the same presence check as before, and
/// it runs only after the populated count already matches `length`.
fn ensure_dense(value: &Value<'_>, len: u64) -> Result<(), BoundaryError> {
    let object = value.as_object().ok_or_else(|| illegal("sparse array"))?;
    let mut index_count = 0u64;
    for key in object.keys::<String>() {
        let key = key.map_err(|error| script_error(value, error))?;
        if array_index(&key).is_some_and(|index| index < len) {
            index_count += 1;
        }
    }
    if index_count != len {
        return Err(illegal("sparse array"));
    }
    let len = usize::try_from(len).map_err(|_| illegal("array length"))?;
    for index in 0..len {
        let key = u32::try_from(index).map_err(|_| illegal("array length"))?;
        let present = object
            .contains_key(key)
            .map_err(|error| script_error(value, error))?;
        if !present {
            return Err(illegal("sparse array"));
        }
    }
    Ok(())
}

/// Canonical JavaScript array index: decimal text, no leading zero, below `2^32 - 1`.
fn array_index(key: &str) -> Option<u64> {
    let bytes = key.as_bytes();
    if bytes.is_empty() || !bytes.iter().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if bytes.len() > 1 && bytes[0] == b'0' {
        return None;
    }
    let index = key.parse::<u64>().ok()?;
    if index >= u32::MAX as u64 {
        None
    } else {
        Some(index)
    }
}

fn number_from_f64(number: f64) -> Result<serde_json::Value, BoundaryError> {
    if !number.is_finite() {
        return Err(illegal("non-finite number"));
    }
    let max = MAX_SAFE_INTEGER as f64;
    if number.fract() == 0.0 && number.abs() <= max {
        let integer = number as i64;
        return Ok(serde_json::Value::Number(integer.into()));
    }
    Number::from_f64(number)
        .map(serde_json::Value::Number)
        .ok_or_else(|| illegal("non-finite number"))
}

fn array_length(value: &Value<'_>) -> Result<u64, BoundaryError> {
    let object = value.as_object().ok_or_else(|| illegal("sparse array"))?;
    let length = object
        .get::<_, Value<'_>>("length")
        .map_err(|error| script_error(value, error))?;
    if let Some(int) = length.as_int() {
        return u64::try_from(int).map_err(|_| illegal("array length"));
    }
    if let Some(number) = length.as_number()
        && number.is_finite()
        && number >= 0.0
        && number.fract() == 0.0
        && number <= u64::MAX as f64
    {
        return Ok(number as u64);
    }
    Err(illegal("array length"))
}

fn reject_symbol_keys(value: &Value<'_>) -> Result<(), BoundaryError> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    match object.own_keys::<Atom<'_>>(Filter::new().symbol()).next() {
        None => Ok(()),
        Some(Ok(_)) => Err(illegal("symbol key")),
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

fn illegal(message: &str) -> BoundaryError {
    BoundaryError::Illegal(message.into())
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

/// Build a JavaScript value on the heap.
///
/// The JSON tree is dismantled iteratively and reassembled with `Array::new`
/// and `Object::new`. `serde_json::to_string` and `JSON.parse` are both
/// recursive, so neither is used.
pub(crate) fn to_js<'js>(ctx: &Ctx<'js>, value: Json) -> Result<Value<'js>, String> {
    enum Work {
        Node(Json),
        Array(usize),
        Object(Vec<String>),
    }

    let mut work = vec![Work::Node(value)];
    let mut built: Vec<Value<'js>> = Vec::new();
    while let Some(item) = work.pop() {
        match item {
            Work::Node(node) => match node.into_value() {
                serde_json::Value::Array(items) => {
                    let len = items.len();
                    work.push(Work::Array(len));
                    for child in items.into_iter().rev() {
                        work.push(Work::Node(Json::new(child)));
                    }
                }
                serde_json::Value::Object(map) => {
                    let mut keys = Vec::with_capacity(map.len());
                    let mut children = Vec::with_capacity(map.len());
                    for (key, child) in map {
                        keys.push(key);
                        children.push(Json::new(child));
                    }
                    work.push(Work::Object(keys));
                    for child in children.into_iter().rev() {
                        work.push(Work::Node(child));
                    }
                }
                leaf => built.push(leaf_to_js(ctx, &leaf)?),
            },
            Work::Array(len) => {
                let array = Array::new(ctx.clone()).map_err(|error| inject_error(ctx, error))?;
                let start = built.len().saturating_sub(len);
                for (index, child) in built.drain(start..).enumerate() {
                    array
                        .set(index, child)
                        .map_err(|error| inject_error(ctx, error))?;
                }
                built.push(array.into_value());
            }
            Work::Object(keys) => {
                let object = Object::new(ctx.clone()).map_err(|error| inject_error(ctx, error))?;
                let start = built.len().saturating_sub(keys.len());
                let children: Vec<_> = built.drain(start..).collect();
                for (key, child) in keys.into_iter().zip(children) {
                    object
                        .set(key, child)
                        .map_err(|error| inject_error(ctx, error))?;
                }
                built.push(object.into_value());
            }
        }
    }
    built
        .pop()
        .ok_or_else(|| inject_error(ctx, Error::new_from_js("JSON", "value")))
}

fn leaf_to_js<'js>(ctx: &Ctx<'js>, value: &serde_json::Value) -> Result<Value<'js>, String> {
    match value {
        serde_json::Value::Null => Ok(Value::new_null(ctx.clone())),
        serde_json::Value::Bool(bit) => Ok(Value::new_bool(ctx.clone(), *bit)),
        serde_json::Value::Number(number) => number_to_js(ctx, number),
        serde_json::Value::String(text) => {
            text.into_js(ctx).map_err(|error| inject_error(ctx, error))
        }
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            Err("host result could not be injected".into())
        }
    }
}

fn number_to_js<'js>(ctx: &Ctx<'js>, number: &Number) -> Result<Value<'js>, String> {
    check_number(number)?;
    let converted = if let Some(integer) = number.as_i64() {
        integer.into_js(ctx)
    } else if let Some(integer) = number.as_u64() {
        integer.into_js(ctx)
    } else if let Some(float) = number.as_f64() {
        float.into_js(ctx)
    } else {
        return Err(unsafe_integer());
    };
    converted.map_err(|error| inject_error(ctx, error))
}

fn inject_error(ctx: &Ctx<'_>, error: Error) -> String {
    let _ = ctx.catch();
    format!("host result could not be injected: {error}")
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

/// Compact JSON bytes, matching `serde_json::to_vec`, without a recursive walk.
pub(crate) fn json_bytes(value: &serde_json::Value) -> Result<Vec<u8>, String> {
    enum Task<'a> {
        Value(&'a serde_json::Value),
        Key(&'a str),
        Byte(u8),
    }

    let mut out = Vec::new();
    let mut stack = vec![Task::Value(value)];
    while let Some(task) = stack.pop() {
        match task {
            Task::Byte(byte) => out.push(byte),
            Task::Key(key) => {
                serde_json::to_writer(&mut out, key).map_err(|error| error.to_string())?;
            }
            Task::Value(serde_json::Value::Array(items)) => {
                out.push(b'[');
                if items.is_empty() {
                    out.push(b']');
                    continue;
                }
                stack.push(Task::Byte(b']'));
                for (index, item) in items.iter().enumerate().rev() {
                    stack.push(Task::Value(item));
                    if index != 0 {
                        stack.push(Task::Byte(b','));
                    }
                }
            }
            Task::Value(serde_json::Value::Object(map)) => {
                out.push(b'{');
                if map.is_empty() {
                    out.push(b'}');
                    continue;
                }
                stack.push(Task::Byte(b'}'));
                for (index, (key, child)) in map.iter().enumerate().rev() {
                    stack.push(Task::Value(child));
                    stack.push(Task::Byte(b':'));
                    stack.push(Task::Key(key));
                    if index != 0 {
                        stack.push(Task::Byte(b','));
                    }
                }
            }
            Task::Value(leaf) => {
                serde_json::to_writer(&mut out, leaf).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::super::Json;
    use super::*;
    use serde_json::json;

    #[test]
    fn json_bytes_matches_serde_for_shallow_values() {
        let values = [
            json!(null),
            json!(true),
            json!(false),
            json!(0),
            json!(-12),
            json!(1.5),
            json!(""),
            json!("a\"b\\c\n"),
            json!("é"),
            json!([]),
            json!({}),
            json!([1, "a", null, true]),
            json!({"b": 1, "a": [2, {"c": "x"}]}),
            json!({"key with \"quote\"": "v"}),
        ];
        for value in values {
            let encoded = json_bytes(&value).expect("encode");
            let standard = serde_json::to_vec(&value).expect("serde");
            assert_eq!(encoded, standard, "{value}");
        }
    }

    #[test]
    fn json_bytes_counts_a_deep_array_on_the_heap() {
        let mut value = Json::new(json!(0));
        for _ in 0..5_000 {
            value = Json::new(serde_json::Value::Array(vec![value.into_value()]));
        }
        let bytes = json_bytes(value.as_value()).expect("encode");
        assert_eq!(bytes.len(), 5_000 * 2 + 1);
        assert_eq!(bytes.iter().filter(|byte| **byte == b'[').count(), 5_000);
        assert_eq!(bytes.iter().filter(|byte| **byte == b']').count(), 5_000);
        assert_eq!(bytes[5_000], b'0');
        assert!(bytes.starts_with(b"[[["));
        assert!(bytes.ends_with(b"]]]"));
        drop(value);
    }
}
