use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use sg_core::{Event, FieldRef, Operator, OperatorError};

fn default_pair_delimiter() -> String {
    " ".to_string()
}
fn default_kv_delimiter() -> String {
    "=".to_string()
}

#[derive(Debug, Deserialize)]
struct KvOpDef {
    parse_from: String,
    parse_to: String,
    #[serde(default = "default_pair_delimiter")]
    pair_delimiter: String,
    #[serde(default = "default_kv_delimiter")]
    kv_delimiter: String,
}

pub struct KvOperator {
    id: String,
    parse_from: FieldRef,
    parse_to: FieldRef,
    pair_delimiter: char,
    kv_delimiter: char,
}

impl KvOperator {
    pub fn from_value(id: &str, value: &Value) -> Result<Self, String> {
        let def: KvOpDef =
            serde_json::from_value(value.clone()).map_err(|e| format!("{id}: {e}"))?;
        let pair_delimiter = def
            .pair_delimiter
            .chars()
            .next()
            .ok_or_else(|| format!("{id}: pair_delimiter must be non-empty"))?;
        let kv_delimiter = def
            .kv_delimiter
            .chars()
            .next()
            .ok_or_else(|| format!("{id}: kv_delimiter must be non-empty"))?;
        Ok(Self {
            id: id.to_string(),
            parse_from: FieldRef::parse(&def.parse_from),
            parse_to: FieldRef::parse(&def.parse_to),
            pair_delimiter,
            kv_delimiter,
        })
    }
}

/// Quote-aware `key=value key2="v v"` scanner. Values wrapped in double
/// quotes may contain the pair delimiter; a backslash inside a quoted
/// value escapes the following character verbatim (no further unescaping
/// beyond that -- sufficient for the syslog-style KV bodies this targets).
fn scan_kv(input: &str, pair_delimiter: char, kv_delimiter: char) -> serde_json::Map<String, Value> {
    let chars: Vec<char> = input.chars().collect();
    let mut result = serde_json::Map::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i] == pair_delimiter {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let key_start = i;
        while i < chars.len() && chars[i] != kv_delimiter && chars[i] != pair_delimiter {
            i += 1;
        }
        let key: String = chars[key_start..i].iter().collect();
        if i >= chars.len() || chars[i] != kv_delimiter || key.is_empty() {
            // Malformed token (no `=` before the next delimiter) -- skip it.
            while i < chars.len() && chars[i] != pair_delimiter {
                i += 1;
            }
            continue;
        }
        i += 1; // skip kv_delimiter

        let value: String = if chars.get(i) == Some(&'"') {
            i += 1;
            let mut v = String::new();
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                v.push(chars[i]);
                i += 1;
            }
            if i < chars.len() {
                i += 1; // skip closing quote
            }
            v
        } else {
            let val_start = i;
            while i < chars.len() && chars[i] != pair_delimiter {
                i += 1;
            }
            chars[val_start..i].iter().collect()
        };
        result.insert(key, Value::String(value));
    }
    result
}

#[async_trait]
impl Operator for KvOperator {
    fn id(&self) -> &str {
        &self.id
    }

    async fn process(&self, mut event: Event) -> Result<Event, (Event, OperatorError)> {
        let input = match self.parse_from.get(&event) {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err((
                    event,
                    OperatorError {
                        operator_id: self.id.clone(),
                        message: "parse_from field is not a string".to_string(),
                    },
                ))
            }
        };

        let extracted = scan_kv(&input, self.pair_delimiter, self.kv_delimiter);
        if extracted.is_empty() {
            return Err((
                event,
                OperatorError {
                    operator_id: self.id.clone(),
                    message: "no key=value pairs found".to_string(),
                },
            ));
        }
        self.parse_to.merge(&mut event, Value::Object(extracted));
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn parses_quoted_and_unquoted_pairs() {
        let cfg = json!({"parse_from": "body", "parse_to": "attributes"});
        let op = KvOperator::from_value("kv", &cfg).unwrap();

        let event = Event::new(bytes::Bytes::from_static(
            br#"user=neo action="mass extract" host=zion1"#,
        ));
        let event = op.process(event).await.unwrap();

        assert_eq!(event.attributes.get("user").unwrap(), "neo");
        assert_eq!(event.attributes.get("action").unwrap(), "mass extract");
        assert_eq!(event.attributes.get("host").unwrap(), "zion1");
    }

    #[tokio::test]
    async fn no_pairs_is_an_error() {
        let cfg = json!({"parse_from": "body", "parse_to": "attributes"});
        let op = KvOperator::from_value("kv", &cfg).unwrap();
        let event = Event::new(bytes::Bytes::from_static(b"just a plain sentence"));
        assert!(op.process(event).await.is_err());
    }
}
