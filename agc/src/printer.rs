//! Output printing — JSON, table, YAML, CSV via --format; field filtering via --fields.

use serde_json::Value;

use crate::error::Result;
use crate::formatter::{OutputFormat, format_value};

/// Print a value using the requested format.
///
/// - `--fields` pre-filters the JSON value before formatting (works for all formats).
/// - `--compact` only applies to JSON output; ignored for table/yaml/csv.
pub fn print_value(value: &Value, fields: Option<&str>, format: OutputFormat, compact: bool) -> Result<()> {
    let filtered = apply_fields(value, fields);
    let out = match (&format, compact) {
        (OutputFormat::Json, true)  => format!("{}\n", serde_json::to_string(&filtered)?),
        (OutputFormat::Json, false) => format!("{}\n", serde_json::to_string_pretty(&filtered)?),
        _                           => format_value(&normalize_for_table(&filtered), &format),
    };
    print!("{out}");
    Ok(())
}

/// Convenience wrapper — always JSON.
/// Used by management commands and schema that are always JSON.
pub fn print_json(value: &Value, fields: Option<&str>, compact: bool) -> Result<()> {
    print_value(value, fields, OutputFormat::Json, compact)
}

/// For table/yaml/csv output of a single JSON object: convert top-level
/// arrays-of-primitives to comma-joined strings.  This prevents gws-cli's
/// `extract_items` heuristic from mistaking a `scopes` or `agents` string-list
/// for a data table.  Arrays-of-objects are left untouched so the formatter
/// can render them as rows.
fn normalize_for_table(value: &Value) -> Value {
    let Value::Object(map) = value else { return value.clone() };
    let mut out = serde_json::Map::new();
    for (k, v) in map {
        let normalized = match v {
            Value::Array(arr) if !arr.is_empty() && arr.iter().all(|i| !i.is_object()) => {
                let joined = arr
                    .iter()
                    .map(|i| match i {
                        Value::String(s) => s.clone(),
                        other => serde_json::to_string(other).unwrap_or_default(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Value::String(joined)
            }
            _ => v.clone(),
        };
        out.insert(k.clone(), normalized);
    }
    Value::Object(out)
}

/// Print a value tagged with agent info as a single compact JSON line (NDJSON).
pub fn print_agent_json(alias: &str, url: &str, value: &Value, fields: Option<&str>) -> Result<()> {
    let mut obj = match value {
        Value::Object(m) => m.clone(),
        other => {
            let mut m = serde_json::Map::new();
            m.insert("result".to_string(), other.clone());
            m
        }
    };
    obj.insert("agent".to_string(), Value::String(alias.to_string()));
    obj.insert("agent_url".to_string(), Value::String(url.to_string()));
    let tagged = Value::Object(obj);
    let filtered = apply_fields(&tagged, fields);
    println!("{}", serde_json::to_string(&filtered)?);
    Ok(())
}

fn apply_fields(value: &Value, fields: Option<&str>) -> Value {
    let Some(f) = fields else { return value.clone() };
    let paths: Vec<&str> = f.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if paths.is_empty() {
        return value.clone();
    }
    if paths.len() == 1 {
        return extract_path(value, paths[0]).unwrap_or(Value::Null);
    }
    let mut out = serde_json::Map::new();
    for path in paths {
        if let Some(v) = extract_path(value, path) {
            let key = path.split('.').next().unwrap_or(path);
            out.insert(key.to_string(), v);
        }
    }
    Value::Object(out)
}

fn extract_path(value: &Value, path: &str) -> Option<Value> {
    let mut current = value;
    for key in path.split('.') {
        current = match current {
            Value::Object(m) => m.get(key)?,
            Value::Array(arr) => {
                let items: Vec<Value> = arr.iter().filter_map(|item| extract_path(item, key)).collect();
                return Some(Value::Array(items));
            }
            _ => return None,
        };
    }
    Some(current.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn no_fields_filter_returns_value_unchanged() {
        let v = json!({"a": 1, "b": 2});
        assert_eq!(apply_fields(&v, None), v);
    }

    #[test]
    fn empty_fields_string_returns_value_unchanged() {
        let v = json!({"a": 1});
        assert_eq!(apply_fields(&v, Some("")), v);
    }

    #[test]
    fn single_field_returns_leaf_value_directly() {
        let v = json!({"text": "hello", "id": "123"});
        assert_eq!(apply_fields(&v, Some("text")), json!("hello"));
    }

    #[test]
    fn single_dotted_path_returns_nested_value() {
        let v = json!({"status": {"state": "completed"}});
        assert_eq!(apply_fields(&v, Some("status.state")), json!("completed"));
    }

    #[test]
    fn multi_field_returns_object_with_requested_keys() {
        let v = json!({"id": "123", "text": "hello", "extra": true});
        let result = apply_fields(&v, Some("id,text"));
        assert_eq!(result["id"], json!("123"));
        assert_eq!(result["text"], json!("hello"));
        assert!(result.get("extra").is_none());
    }

    #[test]
    fn multi_field_with_whitespace_trims_names() {
        let v = json!({"a": 1, "b": 2});
        let result = apply_fields(&v, Some(" a , b "));
        assert_eq!(result["a"], json!(1));
        assert_eq!(result["b"], json!(2));
    }

    #[test]
    fn missing_single_field_returns_null() {
        let v = json!({"a": 1});
        assert_eq!(apply_fields(&v, Some("missing")), json!(null));
    }

    #[test]
    fn array_traversal_collects_field_across_items() {
        let v = json!([{"id": "1"}, {"id": "2"}, {"id": "3"}]);
        assert_eq!(apply_fields(&v, Some("id")), json!(["1", "2", "3"]));
    }

    #[test]
    fn deeply_nested_path() {
        let v = json!({"a": {"b": {"c": 42}}});
        assert_eq!(apply_fields(&v, Some("a.b.c")), json!(42));
    }

    #[test]
    fn path_on_non_object_returns_null() {
        let v = json!({"a": "string"});
        assert_eq!(apply_fields(&v, Some("a.nested")), json!(null));
    }
}
