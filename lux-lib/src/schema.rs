use schemars::{schema_for, JsonSchema};
use serde_json::{Map, Value};

use crate::config::ConfigBuilder;
use crate::project::project_toml::PartialProjectToml;

/// The JSON Schema for the Lux `config.toml` file.
pub fn config_schema() -> schemars::Schema {
    let mut schema = schema_for!(ConfigBuilder);
    schema.insert("title".to_string(), "config.toml".into());
    schema.insert(
        "description".to_string(),
        "Configuration for the Lux package manager. \
         This file lives at the config path reported by `lx config`."
            .into(),
    );
    schema
}

/// The JSON Schema for the Lux `lux.toml` project manifest.
pub fn lux_toml_schema() -> schemars::Schema {
    let mut schema = schema_for!(PartialProjectToml);
    schema.insert("title".to_string(), "lux.toml".into());
    schema.insert(
        "description".to_string(),
        "The TOML manifest for a Lux project."
            .into(),
    );
    schema
}

/// Render a JSON Schema as Markdown.
pub fn to_markdown(schema: &schemars::Schema) -> String {
    let value = serde_json::to_value(schema).unwrap_or(Value::Null);
    let mut out = String::new();
    let defs = definitions(&value);

    if let Some(title) = value.get("title").and_then(Value::as_str) {
        out.push_str("# ");
        out.push_str(title);
        out.push('\n');
    }
    if let Some(desc) = value.get("description").and_then(Value::as_str) {
        out.push('\n');
        out.push_str(desc);
        out.push('\n');
    }

    if let Some(props) = value.get("properties").and_then(Value::as_object) {
        out.push('\n');
        render_properties(&mut out, props, &defs, 2);
    }

    out
}

/// Resolve the map of named subschemas (`definitions` or `$defs`).
fn definitions(value: &Value) -> Map<String, Value> {
    value
        .get("definitions")
        .or_else(|| value.get("$defs"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Resolve `$ref`s and the non-null branch of `anyOf`/`oneOf`, so that
/// `Option<_>` fields resolve to the underlying object schema.
fn deref_schema<'a>(value: &'a Value, defs: &'a Map<String, Value>) -> &'a Value {
    let mut current = value;
    for _ in 0..8 {
        if let Some(reference) = current.get("$ref").and_then(Value::as_str) {
            let name = reference.rsplit('/').next().unwrap_or(reference);
            if let Some(resolved) = defs.get(name) {
                current = resolved;
                continue;
            }
        }
        if let Some(branches) = current
            .get("anyOf")
            .or_else(|| current.get("oneOf"))
            .and_then(Value::as_array)
        {
            let is_nullable = branches
                .iter()
                .any(|b| b.get("type").and_then(Value::as_str) == Some("null"));
            if is_nullable {
                if let Some(branch) = branches
                    .iter()
                    .find(|b| b.get("type").and_then(Value::as_str) != Some("null"))
                {
                    current = branch;
                    continue;
                }
            }
        }
        break;
    }
    current
}

fn render_properties(
    out: &mut String,
    props: &Map<String, Value>,
    defs: &Map<String, Value>,
    level: usize,
) {
    for (name, raw) in props {
        let prop = deref_schema(raw, defs);
        out.push('\n');
        heading(out, level, name);

        if let Some(summary) = type_summary(prop, defs) {
            out.push_str("**Type:** ");
            out.push_str(&summary);
            out.push_str("\n\n");
        }
        let desc = raw
            .get("description")
            .or_else(|| prop.get("description"))
            .and_then(Value::as_str);
        if let Some(desc) = desc {
            out.push_str(desc);
            out.push_str("\n\n");
        }

        let resolved = deref_schema(prop, defs);
        if let Some(nested) = resolved.get("properties").and_then(Value::as_object) {
            render_properties(out, nested, defs, level + 1);
        } else if resolved.get("type").and_then(Value::as_str) == Some("array") {
            if let Some(items) = resolved.get("items") {
                let items = deref_schema(items, defs);
                if let Some(nested) = items.get("properties").and_then(Value::as_object) {
                    render_properties(out, nested, defs, level + 1);
                }
            }
        }
    }
}

fn heading(out: &mut String, level: usize, name: &str) {
    for _ in 0..level {
        out.push('#');
    }
    out.push(' ');
    out.push_str(name);
    out.push('\n');
}

/// A human-readable summary of a (sub)schema's type.
fn type_summary(value: &Value, defs: &Map<String, Value>) -> Option<String> {
    let value = deref_schema(value, defs);

    if let Some(values) = value.get("enum").and_then(Value::as_array) {
        let vals: Vec<String> = values
            .iter()
            .filter_map(Value::as_str)
            .map(|s| format!("`{s}`"))
            .collect();
        if !vals.is_empty() {
            return Some(format!("one of {}", vals.join(", ")));
        }
    }

    if let Some(variants) = value.get("anyOf").or_else(|| value.get("oneOf")) {
        if let Some(variants) = variants.as_array() {
            let parts: Vec<String> = variants
                .iter()
                .filter_map(|v| type_summary(v, defs))
                .filter(|p| p != "null")
                .collect();
            if !parts.is_empty() {
                return Some(parts.join(" or "));
            }
        }
    }

    if let Some(items) = value.get("items") {
        if let Some(item) = type_summary(items, defs) {
            return Some(format!("array of {item}"));
        }
        return Some("array".to_string());
    }

    if let Some(additional) = value.get("additionalProperties") {
        if let Some(inner) = type_summary(additional, defs) {
            return Some(format!("map of `string` to {inner}"));
        }
    }

    match value.get("type") {
        Some(Value::String(t)) => Some((*t).clone()),
        Some(Value::Array(types)) => {
            let parts: Vec<String> = types
                .iter()
                .filter_map(Value::as_str)
                .filter(|t| *t != "null")
                .map(String::from)
                .collect();
            Some(parts.join(" or "))
        }
        _ => None,
    }
}

// Types describing the user-facing shape of fields whose serde
// representation differs from their Rust type,
// because they use custom `Deserialize` implementations.
//
// These exist only to feed `#[schemars(with = ...)]` annotations.

/// A single `[dependencies]`-style entry: either a bare version requirement
/// or a detailed table.
#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(untagged)]
pub(crate) enum DependencyEntry {
    Simple(String),
    Detailed(DependencyTableEntry),
}

#[allow(dead_code)]
#[derive(JsonSchema)]
pub(crate) struct DependencyTableEntry {
    pub version: String,
    pub opt: Option<bool>,
    pub pin: Option<bool>,
    pub git: Option<String>,
    pub path: Option<String>,
    pub rev: Option<String>,
}

/// A `build.modules` entry: a source path, a list of source paths, or a
/// table describing a compiled module.
#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(untagged)]
pub(crate) enum ModuleSpec {
    SourcePath(String),
    SourcePaths(Vec<String>),
    ModulePaths(ModulePathsSpec),
}

#[allow(dead_code)]
#[derive(JsonSchema)]
pub(crate) struct ModulePathsSpec {
    pub sources: Vec<String>,
    pub libraries: Vec<String>,
    pub defines: Vec<String>,
    pub incdirs: Vec<String>,
    pub libdirs: Vec<String>,
}
