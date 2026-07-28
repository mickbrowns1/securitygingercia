use crate::event::Event;
use serde_json::Value;

/// Addresses a field an operator reads from or writes to. Mirrors the
/// `parse_from`/`parse_to` idiom used by OTel's stanza operators, without
/// inventing a full expression language: just three roots, each optionally
/// followed by a dotted path into a nested object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldRef {
    Body,
    Attribute(String),
    Resource(String),
}

impl FieldRef {
    pub fn parse(s: &str) -> Self {
        if s == "body" {
            FieldRef::Body
        } else if let Some(rest) = s.strip_prefix("attributes.") {
            FieldRef::Attribute(rest.to_string())
        } else if s == "attributes" {
            FieldRef::Attribute(String::new())
        } else if let Some(rest) = s.strip_prefix("resource.") {
            FieldRef::Resource(rest.to_string())
        } else if s == "resource" {
            FieldRef::Resource(String::new())
        } else {
            // Bare names default to attributes, matching stanza's convention
            // that unqualified fields live in the record's extracted attributes.
            FieldRef::Attribute(s.to_string())
        }
    }

    pub fn get<'a>(&self, event: &'a Event) -> Option<&'a Value> {
        match self {
            FieldRef::Body => Some(&event.body),
            FieldRef::Attribute(path) => get_path(&event.attributes, path),
            FieldRef::Resource(path) => get_path(&event.resource, path),
        }
    }

    pub fn set(&self, event: &mut Event, value: Value) {
        match self {
            FieldRef::Body => event.body = value,
            FieldRef::Attribute(path) => set_path(&mut event.attributes, path, value),
            FieldRef::Resource(path) => set_path(&mut event.resource, path, value),
        }
    }

    /// Merge `value` into the target: if the target is `attributes`/`resource`
    /// and `value` is an object, its keys are merged in; otherwise behaves
    /// like `set`.
    pub fn merge(&self, event: &mut Event, value: Value) {
        match (self, &value) {
            (FieldRef::Attribute(path), Value::Object(map)) if path.is_empty() => {
                event.attributes.extend(map.clone());
            }
            (FieldRef::Resource(path), Value::Object(map)) if path.is_empty() => {
                event.resource.extend(map.clone());
            }
            _ => self.set(event, value),
        }
    }

    pub fn remove(&self, event: &mut Event) {
        match self {
            FieldRef::Body => event.body = Value::Null,
            FieldRef::Attribute(path) => remove_path(&mut event.attributes, path),
            FieldRef::Resource(path) => remove_path(&mut event.resource, path),
        }
    }
}

fn get_path<'a>(map: &'a serde_json::Map<String, Value>, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return None;
    }
    let mut segments = path.split('.');
    let first = segments.next()?;
    let mut current = map.get(first)?;
    for seg in segments {
        current = current.as_object()?.get(seg)?;
    }
    Some(current)
}

fn set_path(map: &mut serde_json::Map<String, Value>, path: &str, value: Value) {
    if path.is_empty() {
        if let Value::Object(obj) = value {
            map.extend(obj);
        }
        return;
    }
    let segments: Vec<&str> = path.split('.').collect();
    let mut current = map;
    for seg in &segments[..segments.len() - 1] {
        let entry = current
            .entry(seg.to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(serde_json::Map::new());
        }
        current = entry.as_object_mut().unwrap();
    }
    current.insert(segments[segments.len() - 1].to_string(), value);
}

fn remove_path(map: &mut serde_json::Map<String, Value>, path: &str) {
    if path.is_empty() {
        map.clear();
        return;
    }
    let segments: Vec<&str> = path.split('.').collect();
    let mut current = map;
    for seg in &segments[..segments.len() - 1] {
        match current.get_mut(*seg).and_then(|v| v.as_object_mut()) {
            Some(next) => current = next,
            None => return,
        }
    }
    current.remove(segments[segments.len() - 1]);
}
