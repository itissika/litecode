//! JSON value whose Drop, Clone, Debug, and PartialEq walk on the heap.
//!
//! `serde_json` 1.0.150 derives those four for [`serde_json::Value`]. A
//! 20,000-deep tree then overflows the thread stack (`0xC00000FD`) when the
//! caller simply drops it. This wrapper peels containers onto an explicit
//! stack, the same way a manual dismantle would, so the derived impls never run
//! on a deep tree.

use std::collections::BTreeMap;
use std::fmt;
use std::mem::ManuallyDrop;
use std::ops::Deref;

use serde_json::Value;

/// Owned JSON tree. Dropping, cloning, comparing, and debugging it do not use
/// the thread stack to follow nesting.
pub struct Json {
    value: ManuallyDrop<Value>,
}

impl Json {
    pub fn new(value: Value) -> Self {
        Self {
            value: ManuallyDrop::new(value),
        }
    }

    pub fn as_value(&self) -> &Value {
        &self.value
    }

    /// Hand the inner tree to the caller. Drop will not peel it again.
    pub fn into_value(self) -> Value {
        let mut this = ManuallyDrop::new(self);
        // SAFETY: `this` is not dropped, so the inner value is taken once.
        unsafe { ManuallyDrop::take(&mut this.value) }
    }

    pub(crate) fn from_array(items: Vec<Json>) -> Self {
        let values = items.into_iter().map(Json::into_value).collect();
        Self::new(Value::Array(values))
    }

    pub(crate) fn from_object(map: BTreeMap<String, Json>) -> Self {
        let mut values = serde_json::Map::new();
        for (key, child) in map {
            values.insert(key, child.into_value());
        }
        Self::new(Value::Object(values))
    }
}

impl Drop for Json {
    fn drop(&mut self) {
        // SAFETY: `value` is initialized until this take, and Drop runs once.
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        dismantle(value);
    }
}

impl Clone for Json {
    fn clone(&self) -> Self {
        Self::new(clone_value(self.as_value()))
    }
}

impl fmt::Debug for Json {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_value(self.as_value(), formatter)
    }
}

impl Deref for Json {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        self.as_value()
    }
}

impl From<Value> for Json {
    fn from(value: Value) -> Self {
        Self::new(value)
    }
}

/// Peel containers onto a heap stack. Leaves drop normally.
fn dismantle(value: Value) {
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::Array(items) => stack.extend(items),
            Value::Object(map) => stack.extend(map.into_values()),
            _ => {}
        }
    }
}

fn clone_value(root: &Value) -> Value {
    enum Task<'a> {
        Value(&'a Value),
        EndArray(usize),
        EndObject(Vec<String>),
    }

    let mut tasks = vec![Task::Value(root)];
    let mut built: Vec<Json> = Vec::new();
    while let Some(task) = tasks.pop() {
        match task {
            Task::Value(Value::Array(items)) => {
                tasks.push(Task::EndArray(items.len()));
                for item in items.iter().rev() {
                    tasks.push(Task::Value(item));
                }
            }
            Task::Value(Value::Object(map)) => {
                let mut keys = Vec::with_capacity(map.len());
                let mut children = Vec::with_capacity(map.len());
                for (key, child) in map {
                    keys.push(key.clone());
                    children.push(child);
                }
                tasks.push(Task::EndObject(keys));
                for child in children.into_iter().rev() {
                    tasks.push(Task::Value(child));
                }
            }
            Task::Value(Value::Null) => built.push(Json::new(Value::Null)),
            Task::Value(Value::Bool(bit)) => built.push(Json::new(Value::Bool(*bit))),
            Task::Value(Value::Number(number)) => {
                built.push(Json::new(Value::Number(number.clone())));
            }
            Task::Value(Value::String(text)) => built.push(Json::new(Value::String(text.clone()))),
            Task::EndArray(len) => {
                let start = built.len().saturating_sub(len);
                let items: Vec<Json> = built.drain(start..).collect();
                built.push(Json::from_array(items));
            }
            Task::EndObject(keys) => {
                let start = built.len().saturating_sub(keys.len());
                let children: Vec<Json> = built.drain(start..).collect();
                let mut map = BTreeMap::new();
                for (key, child) in keys.into_iter().zip(children) {
                    map.insert(key, child);
                }
                built.push(Json::from_object(map));
            }
        }
    }
    built.pop().map(Json::into_value).unwrap_or(Value::Null)
}

fn fmt_value(root: &Value, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    enum Tok<'a> {
        Value(&'a Value),
        Key(&'a str),
        Text(&'static str),
    }

    let mut stack = vec![Tok::Value(root)];
    while let Some(token) = stack.pop() {
        match token {
            Tok::Text(text) => formatter.write_str(text)?,
            Tok::Key(key) => write!(formatter, "{key:?}")?,
            Tok::Value(Value::Array(items)) => {
                formatter.write_str("Array [")?;
                if items.is_empty() {
                    formatter.write_str("]")?;
                    continue;
                }
                stack.push(Tok::Text("]"));
                for (index, item) in items.iter().enumerate().rev() {
                    stack.push(Tok::Value(item));
                    if index != 0 {
                        stack.push(Tok::Text(", "));
                    }
                }
            }
            Tok::Value(Value::Object(map)) => {
                formatter.write_str("Object {")?;
                if map.is_empty() {
                    formatter.write_str("}")?;
                    continue;
                }
                stack.push(Tok::Text("}"));
                for (index, (key, child)) in map.iter().enumerate().rev() {
                    stack.push(Tok::Value(child));
                    stack.push(Tok::Text(": "));
                    stack.push(Tok::Key(key));
                    if index != 0 {
                        stack.push(Tok::Text(", "));
                    }
                }
            }
            Tok::Value(leaf) => write!(formatter, "{leaf:?}")?,
        }
    }
    Ok(())
}

impl PartialEq for Json {
    fn eq(&self, other: &Self) -> bool {
        values_eq(self.as_value(), other.as_value())
    }
}

fn values_eq(left: &Value, right: &Value) -> bool {
    let mut stack = vec![(left, right)];
    while let Some((left, right)) = stack.pop() {
        match (left, right) {
            (Value::Null, Value::Null) => {}
            (Value::Bool(left), Value::Bool(right)) if left == right => {}
            (Value::Number(left), Value::Number(right)) if left == right => {}
            (Value::String(left), Value::String(right)) if left == right => {}
            (Value::Array(left), Value::Array(right)) if left.len() == right.len() => {
                stack.extend(left.iter().zip(right));
            }
            (Value::Object(left), Value::Object(right)) if left.len() == right.len() => {
                for ((left_key, left_value), (right_key, right_value)) in left.iter().zip(right) {
                    if left_key != right_key {
                        return false;
                    }
                    stack.push((left_value, right_value));
                }
            }
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    
    

}
