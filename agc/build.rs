//! Build script:
//!   1. Compiles a2a.proto → FileDescriptorSet via protoc
//!   2. Loads it into a prost-reflect DescriptorPool
//!   3. Walks descriptors to generate JSON Schema for three key message types
//!   4. Writes schema_send.json / schema_task.json / schema_card.json to OUT_DIR

use std::collections::HashSet;
use std::path::PathBuf;

use prost_reflect::{Cardinality, DescriptorPool, FieldDescriptor, Kind, MessageDescriptor};
use serde_json::{Map, Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // BUILD_ENV → compile-time default host
    let host = match std::env::var("BUILD_ENV").as_deref() {
        Ok("dev") => "dev.genai.stargate.toyota",
        Ok("stg") => "stg.genai.stargate.toyota",
        _ => "genai.stargate.toyota",
    };
    println!("cargo:rustc-env=AGC_DEFAULT_HOST={host}");
    println!("cargo:rerun-if-env-changed=BUILD_ENV");

    // Proto → JSON Schema
    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let proto_root = PathBuf::from("../a2a-rs/a2a-pb/proto");
    let descriptor_path = out_dir.join("a2a-descriptor.bin");

    println!("cargo:rerun-if-changed=../a2a-rs/a2a-pb/proto/a2a.proto");

    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    let status = std::process::Command::new(protoc)
        .arg("--include_imports")
        .arg("--include_source_info")
        .arg(format!(
            "--descriptor_set_out={}",
            descriptor_path.display()
        ))
        .arg(format!("-I{}", proto_root.display()))
        .arg(proto_root.join("a2a.proto"))
        .status()?;
    if !status.success() {
        return Err("protoc failed".into());
    }

    let mut pool = DescriptorPool::new();
    pool.decode_file_descriptor_set(std::fs::read(&descriptor_path)?.as_slice())?;

    for (msg, file) in [
        ("lf.a2a.v1.SendMessageRequest", "schema_send.json"),
        ("lf.a2a.v1.Task", "schema_task.json"),
        ("lf.a2a.v1.AgentCard", "schema_card.json"),
    ] {
        let schema = generate(&pool, msg)?;
        std::fs::write(out_dir.join(file), serde_json::to_string_pretty(&schema)?)?;
    }

    Ok(())
}

// ── Schema generation ─────────────────────────────────────────────────

fn generate(pool: &DescriptorPool, name: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let msg = pool
        .get_message_by_name(name)
        .ok_or_else(|| format!("message not found: {name}"))?;
    let mut defs: Map<String, Value> = Map::new();
    let mut resolving = HashSet::new();
    let mut schema = msg_schema(&msg, &mut defs, &mut resolving);
    schema["$schema"] = json!("https://json-schema.org/draft-07/schema");
    if !defs.is_empty() {
        schema["$defs"] = Value::Object(defs);
    }
    Ok(schema)
}

fn msg_schema(
    msg: &MessageDescriptor,
    defs: &mut Map<String, Value>,
    resolving: &mut HashSet<String>,
) -> Value {
    let mut properties: Map<String, Value> = Map::new();
    for field in msg.fields() {
        let fs = field_schema(&field, defs, resolving);
        properties.insert(field.json_name().to_string(), fs);
    }
    let mut schema = json!({
        "title": msg.name(),
        "type": "object",
        "properties": Value::Object(properties),
    });
    let c = msg_comment(msg);
    if !c.is_empty() {
        schema["description"] = json!(c);
    }
    schema
}

fn field_schema(
    field: &FieldDescriptor,
    defs: &mut Map<String, Value>,
    resolving: &mut HashSet<String>,
) -> Value {
    let mut schema = if field.is_map() {
        // Map field — get the value type from the synthetic map-entry message.
        if let Kind::Message(entry) = field.kind() {
            if let Some(val) = entry.fields().find(|f| f.number() == 2) {
                let val_schema = kind_schema(val.kind(), defs, resolving);
                json!({ "type": "object", "additionalProperties": val_schema })
            } else {
                json!({ "type": "object" })
            }
        } else {
            json!({ "type": "object" })
        }
    } else if field.cardinality() == Cardinality::Repeated {
        let items = kind_schema(field.kind(), defs, resolving);
        json!({ "type": "array", "items": items })
    } else {
        kind_schema(field.kind(), defs, resolving)
    };
    let c = field_comment(field);
    if !c.is_empty() {
        schema["description"] = json!(c);
    }
    schema
}

fn kind_schema(
    kind: Kind,
    defs: &mut Map<String, Value>,
    resolving: &mut HashSet<String>,
) -> Value {
    match kind {
        Kind::String => json!({ "type": "string" }),
        Kind::Bytes => json!({ "type": "string", "contentEncoding": "base64" }),
        Kind::Bool => json!({ "type": "boolean" }),
        Kind::Float | Kind::Double => json!({ "type": "number" }),
        Kind::Int32
        | Kind::Sint32
        | Kind::Sfixed32
        | Kind::Uint32
        | Kind::Fixed32
        | Kind::Int64
        | Kind::Sint64
        | Kind::Sfixed64
        | Kind::Uint64
        | Kind::Fixed64 => json!({ "type": "integer" }),
        Kind::Enum(e) => {
            let vals: Vec<String> = e.values().map(|v| v.name().to_string()).collect();
            json!({ "type": "string", "enum": vals })
        }
        Kind::Message(msg) => msg_ref(&msg, defs, resolving),
    }
}

fn msg_ref(
    msg: &MessageDescriptor,
    defs: &mut Map<String, Value>,
    resolving: &mut HashSet<String>,
) -> Value {
    // Inline well-known types rather than creating $defs entries for them.
    match msg.full_name() {
        "google.protobuf.Struct" => return json!({ "type": "object" }),
        "google.protobuf.Value" => return json!({}),
        "google.protobuf.Timestamp" => return json!({ "type": "string", "format": "date-time" }),
        "google.protobuf.ListValue" => return json!({ "type": "array" }),
        _ => {}
    }
    let name = msg.name().to_string();
    if resolving.contains(&name) {
        return json!({ "$ref": format!("#/$defs/{name}") });
    }
    if !defs.contains_key(&name) {
        resolving.insert(name.clone());
        let def = msg_schema(msg, defs, resolving);
        resolving.remove(&name);
        defs.insert(name.clone(), def);
    }
    json!({ "$ref": format!("#/$defs/{name}") })
}

// ── Comment helpers ───────────────────────────────────────────────────

fn msg_comment(msg: &MessageDescriptor) -> String {
    comment_at(msg.parent_file_descriptor_proto(), msg.path())
}

fn field_comment(field: &FieldDescriptor) -> String {
    comment_at(field.parent_file().file_descriptor_proto(), field.path())
}

fn comment_at(file: &prost_types::FileDescriptorProto, path: &[i32]) -> String {
    file.source_code_info
        .as_ref()
        .and_then(|sci| {
            sci.location
                .iter()
                .find(|loc| loc.path == path)
                .and_then(|loc| {
                    let c = loc.leading_comments().trim().to_string();
                    if c.is_empty() { None } else { Some(c) }
                })
        })
        .unwrap_or_default()
}
