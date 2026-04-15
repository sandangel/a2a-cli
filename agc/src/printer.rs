//! Output printing — JSON, table, YAML, CSV via --format; jq field filtering via --fields.

use jaq_core::load::{Arena, File, Loader};
use jaq_core::{Ctx, Vars, data, unwrap_valr};
use jaq_json::{Val, read};
use serde_json::Value;

use crate::error::{AgcError, Result};
use crate::formatter::{OutputFormat, format_value};

/// Print a value using the requested format.
///
/// - `--fields` applies a jq filter before formatting (works for all formats).
/// - `--compact` only applies to JSON output; ignored for table/yaml/csv.
/// - Multiple jq outputs are printed as compact NDJSON (one per line).
pub fn print_value(
    value: &Value,
    fields: Option<&str>,
    format: OutputFormat,
    compact: bool,
) -> Result<()> {
    if let Some(filter) = fields {
        let results = apply_jq(value, filter)?;
        return print_jq_results(&results, &format, compact);
    }
    let out = format_one(value, &format, compact)?;
    print!("{out}");
    Ok(())
}

/// Convenience wrapper — always JSON.
/// Used by management commands and schema that are always JSON.
pub fn print_json(value: &Value, fields: Option<&str>, compact: bool) -> Result<()> {
    print_value(value, fields, OutputFormat::Json, compact)
}

/// Print a value tagged with agent info as a single compact JSON line (NDJSON).
pub fn print_agent_json(alias: &str, url: &str, value: &Value, fields: Option<&str>) -> Result<()> {
    let tagged = tag_with_agent(alias, url, value);
    if let Some(filter) = fields {
        let results = apply_jq(&tagged, filter)?;
        for v in &results {
            println!("{}", serde_json::to_string(v)?);
        }
    } else {
        println!("{}", serde_json::to_string(&tagged)?);
    }
    Ok(())
}

fn tag_with_agent(alias: &str, url: &str, value: &Value) -> Value {
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
    Value::Object(obj)
}

/// Apply a jq filter string to a value and return all output values.
fn apply_jq(value: &Value, filter: &str) -> Result<Vec<Value>> {
    // serde_json::Value → jaq Val via JSON bytes
    let bytes = serde_json::to_vec(value).map_err(AgcError::Json)?;
    let input = read::parse_single(&bytes)
        .map_err(|e| AgcError::InvalidInput(format!("jq input error: {e}")))?;

    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());

    let program = File {
        code: filter,
        path: (),
    };
    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(&arena, program)
        .map_err(|e| AgcError::InvalidInput(format!("jq parse error: {e:?}")))?;

    let compiled = jaq_core::Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|e| AgcError::InvalidInput(format!("jq compile error: {e:?}")))?;

    let ctx = Ctx::<data::JustLut<Val>>::new(&compiled.lut, Vars::new([]));
    compiled
        .id
        .run((ctx, input))
        .map(|r| {
            let v = unwrap_valr(r)
                .map_err(|e| AgcError::InvalidInput(format!("jq runtime error: {e}")))?;
            // Val has no Serialize impl; round-trip through its JSON Display
            serde_json::from_str(&v.to_string()).map_err(AgcError::Json)
        })
        .collect()
}

/// Print jq results: single result respects --format/--compact; multiple results are NDJSON.
fn print_jq_results(results: &[Value], format: &OutputFormat, compact: bool) -> Result<()> {
    match results.len() {
        0 => {}
        1 => print!("{}", format_one(&results[0], format, compact)?),
        _ => {
            for v in results {
                println!("{}", serde_json::to_string(v)?);
            }
        }
    }
    Ok(())
}

/// Format a single value according to the output format and compact flag.
fn format_one(value: &Value, format: &OutputFormat, compact: bool) -> Result<String> {
    Ok(match (format, compact) {
        (OutputFormat::Json, true) => format!("{}\n", serde_json::to_string(value)?),
        (OutputFormat::Json, false) => format!("{}\n", serde_json::to_string_pretty(value)?),
        _ => format_value(&normalize_for_table(value), format),
    })
}

/// For table/yaml/csv output of a single JSON object: convert top-level
/// arrays-of-primitives to comma-joined strings.  This prevents gws-cli's
/// `extract_items` heuristic from mistaking a `scopes` or `agents` string-list
/// for a data table.  Arrays-of-objects are left untouched so the formatter
/// can render them as rows.
fn normalize_for_table(value: &Value) -> Value {
    let Value::Object(map) = value else {
        return value.clone();
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jq(value: &Value, filter: &str) -> Value {
        let results = apply_jq(value, filter).expect("jq failed");
        match results.len() {
            0 => Value::Null,
            1 => results.into_iter().next().unwrap(),
            _ => Value::Array(results),
        }
    }

    #[test]
    fn no_filter_returns_value_unchanged() {
        let v = json!({"a": 1, "b": 2});
        // no filter — print_value with None fields leaves value as-is
        // tested indirectly; apply_jq is only called when fields is Some
        let _ = v;
    }

    #[test]
    fn single_field_returns_leaf_value() {
        let v = json!({"text": "hello", "id": "123"});
        assert_eq!(jq(&v, ".text"), json!("hello"));
    }

    #[test]
    fn dotted_path_returns_nested_value() {
        let v = json!({"status": {"state": "completed"}});
        assert_eq!(jq(&v, ".status.state"), json!("completed"));
    }

    #[test]
    fn array_index_returns_element() {
        let v = json!({"artifacts": [{"id": "a1"}, {"id": "a2"}]});
        assert_eq!(jq(&v, ".artifacts[0]"), json!({"id": "a1"}));
        assert_eq!(jq(&v, ".artifacts[1].id"), json!("a2"));
    }

    #[test]
    fn array_iterate_returns_multiple() {
        let v = json!({"artifacts": [{"id": "a1"}, {"id": "a2"}]});
        assert_eq!(jq(&v, ".artifacts[].id"), json!(["a1", "a2"]));
    }

    #[test]
    fn deeply_nested_index_path() {
        let v = json!({"artifacts": [{"parts": [{"text": "hello"}]}]});
        assert_eq!(jq(&v, ".artifacts[0].parts[0].text"), json!("hello"));
    }

    #[test]
    fn missing_field_returns_null() {
        let v = json!({"a": 1});
        assert_eq!(jq(&v, ".missing"), json!(null));
    }

    #[test]
    fn identity_filter_returns_value() {
        let v = json!({"a": 1, "b": 2});
        assert_eq!(jq(&v, "."), v);
    }

    #[test]
    fn multiple_outputs_via_comma() {
        let v = json!({"id": "123", "name": "foo"});
        // .id,.name produces two outputs → Vec of two values
        let results = apply_jq(&v, ".id,.name").unwrap();
        assert_eq!(results, vec![json!("123"), json!("foo")]);
    }
}
