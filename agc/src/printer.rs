//! JSON printing with --fields filtering.

use serde_json::Value;

use crate::error::Result;

/// Print a JSON value to stdout with optional field filtering.
pub fn print_json(value: &Value, fields: Option<&str>, compact: bool) -> Result<()> {
    let filtered = apply_fields(value, fields);
    let out = if compact {
        serde_json::to_string(&filtered)?
    } else {
        serde_json::to_string_pretty(&filtered)?
    };
    println!("{out}");
    Ok(())
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
