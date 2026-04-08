use anyhow::{Result, anyhow, bail};
use indexmap::IndexMap;
use serde_json::{Map, Number, Value, json};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaType {
    String,
    Number,
    Integer,
    Boolean,
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FormFieldKind {
    #[default]
    Scalar,
    OneOfSelector {
        branch_count: usize,
    },
    ArrayPlaceholder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    pub path: Vec<String>,
    pub key: String,
    pub label: String,
    pub description: Option<String>,
    pub schema_type: SchemaType,
    pub enum_options: Option<Vec<String>>,
    pub multiline: bool,
    pub required: bool,
    pub edit_buffer: String,
    pub kind: FormFieldKind,
}

/// Stable key for [oneOf](https://json-schema.org/understanding-json-schema/reference/combining.html#oneOf) branch overrides.
pub fn form_path_key(path: &[String]) -> String {
    Value::Array(path.iter().cloned().map(Value::String).collect()).to_string()
}

#[derive(Clone, Copy)]
pub struct ResolveCtx<'a> {
    pub root: &'a Value,
    pub instance: Option<&'a Value>,
    pub choices: Option<&'a HashMap<String, usize>>,
}

pub fn default_value_for_schema(root: &Value, schema: &Value) -> Result<Value> {
    let ctx = ResolveCtx {
        root,
        instance: None,
        choices: None,
    };
    default_value_for_schema_ctx(&ctx, schema, &[])
}

pub fn default_value_at_path(
    root: &Value,
    path: &[String],
    choices: Option<&HashMap<String, usize>>,
) -> Result<Value> {
    let ctx = ResolveCtx {
        root,
        instance: None,
        choices,
    };
    let property_schema = if path.is_empty() {
        root
    } else {
        let parent_path = &path[..path.len() - 1];
        let key = path.last().expect("non-empty path");
        let parent_schema = resolve_schema_at_path_with(parent_path, ResolveCtx {
            root,
            instance: None,
            choices,
        })?;
        parent_schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(key))
            .ok_or_else(|| anyhow!("schema path not found: {}", path.join(".")))?
    };
    default_value_for_schema_ctx(&ctx, property_schema, path)
}

pub fn build_form_fields(root: &Value, schema: &Value, value: &Value) -> Vec<FormField> {
    build_form_fields_with(root, schema, value, None)
}

pub fn build_form_fields_with(
    root: &Value,
    schema: &Value,
    value: &Value,
    choices: Option<&HashMap<String, usize>>,
) -> Vec<FormField> {
    let ctx = ResolveCtx {
        root,
        instance: Some(value),
        choices,
    };
    let mut fields = Vec::new();
    flatten_fields(&ctx, schema, value, &[], false, &mut fields);
    fields
}

pub fn replace_json_at_path(target: &mut Value, path: &[String], value: Value) -> Result<()> {
    set_value_at_path(target, path, value)
}

pub fn append_array_item(
    current: &Value,
    root: &Value,
    array_path: &[String],
    choices: Option<&HashMap<String, usize>>,
) -> Result<Value> {
    let ctx = ResolveCtx {
        root,
        instance: Some(current),
        choices,
    };
    let array_schema = resolve_schema_at_path_with(array_path, ctx)?;
    let array_schema = resolve_schema_in_context(ctx, array_schema, array_path)?;
    let array = value_at_path(current, array_path)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("array path not found: {}", array_path.join(".")))?;
    if let Some(max_items) = array_schema.get("maxItems").and_then(Value::as_u64) {
        if array.len() >= max_items as usize {
            bail!("maxItems reached for {}", array_path.join("."));
        }
    }

    let next_index = array.len();
    let next_item =
        if let Some(prefix_items) = array_schema.get("prefixItems").and_then(Value::as_array) {
            if let Some(prefix_schema) = prefix_items.get(next_index) {
                default_value_for_schema_ctx(
                    &ResolveCtx {
                        root,
                        instance: None,
                        choices,
                    },
                    prefix_schema,
                    &item_path(array_path, next_index),
                )?
            } else if let Some(items_schema) = array_schema.get("items") {
                default_value_for_schema_ctx(
                    &ResolveCtx {
                        root,
                        instance: None,
                        choices,
                    },
                    items_schema,
                    &item_path(array_path, next_index),
                )?
            } else {
                bail!(
                    "array does not allow additional items: {}",
                    array_path.join(".")
                );
            }
        } else if let Some(items_schema) = array_schema.get("items") {
            default_value_for_schema_ctx(
                &ResolveCtx {
                    root,
                    instance: None,
                    choices,
                },
                items_schema,
                &item_path(array_path, next_index),
            )?
        } else {
            bail!(
                "array does not allow additional items: {}",
                array_path.join(".")
            );
        };

    let mut next = current.clone();
    push_value_at_path(&mut next, array_path, next_item)?;
    Ok(next)
}

pub fn remove_array_item(
    current: &Value,
    root: &Value,
    array_path: &[String],
    index: usize,
    choices: Option<&HashMap<String, usize>>,
) -> Result<Value> {
    let ctx = ResolveCtx {
        root,
        instance: Some(current),
        choices,
    };
    let array_schema = resolve_schema_at_path_with(array_path, ctx)?;
    let array_schema = resolve_schema_in_context(ctx, array_schema, array_path)?;
    let array = value_at_path(current, array_path)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("array path not found: {}", array_path.join(".")))?;
    let min_items = array_schema
        .get("minItems")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    if array.len() <= min_items {
        bail!("minItems reached for {}", array_path.join("."));
    }
    if index >= array.len() {
        bail!("array index out of bounds: {}", index);
    }

    let mut next = current.clone();
    remove_value_at_path(&mut next, array_path, index)?;
    Ok(next)
}

fn item_path(array_path: &[String], index: usize) -> Vec<String> {
    let mut p = array_path.to_vec();
    p.push(index.to_string());
    p
}

fn resolve_ref_only<'a>(root: &'a Value, mut schema: &'a Value) -> Result<&'a Value> {
    while let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        schema = resolve_internal_ref(root, reference)?;
    }
    Ok(schema)
}

fn infer_one_of_branch_index(root: &Value, one_of_schema: &Value, value: &Value) -> usize {
    let Some(alts) = one_of_schema.get("oneOf").and_then(Value::as_array) else {
        return 0;
    };
    if alts.is_empty() {
        return 0;
    }
    let Some(obj) = value.as_object() else {
        return 0;
    };
    let mut common: Option<HashSet<String>> = None;
    for alt in alts {
        let Ok(resolved) = resolve_ref_only(root, alt) else {
            continue;
        };
        let Some(props) = resolved.get("properties").and_then(Value::as_object) else {
            common = None;
            break;
        };
        let with_const: HashSet<String> = props
            .iter()
            .filter(|(_, sub)| sub.get("const").is_some())
            .map(|(k, _)| k.clone())
            .collect();
        common = Some(match &common {
            None => with_const,
            Some(prev) => prev.intersection(&with_const).cloned().collect(),
        });
    }
    let Some(keys) = common else {
        return 0;
    };
    for key in keys {
        if let Some(v) = obj.get(&key) {
            for (i, alt) in alts.iter().enumerate() {
                let Ok(resolved) = resolve_ref_only(root, alt) else {
                    continue;
                };
                let branch_const = resolved
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|p| p.get(&key))
                    .and_then(|s| s.get("const"));
                if branch_const == Some(v) {
                    return i;
                }
            }
        }
    }
    0
}

fn pick_one_of_index(ctx: ResolveCtx<'_>, one_of_schema: &Value, value_path: &[String]) -> usize {
    let Some(alts) = one_of_schema.get("oneOf").and_then(Value::as_array) else {
        return 0;
    };
    let len = alts.len();
    if len == 0 {
        return 0;
    }
    let from_map = ctx
        .choices
        .and_then(|choices| choices.get(&form_path_key(value_path)))
        .copied();
    if let Some(idx) = from_map {
        return idx.min(len - 1);
    }
    if let Some(instance) = ctx.instance {
        if let Some(v) = value_at_path(instance, value_path) {
            return infer_one_of_branch_index(ctx.root, one_of_schema, v).min(len - 1);
        }
    }
    0
}

fn resolve_schema_in_context<'a>(
    ctx: ResolveCtx<'a>,
    schema: &'a Value,
    value_path: &[String],
) -> Result<&'a Value> {
    let schema = resolve_ref_only(ctx.root, schema)?;
    if let Some(alternatives) = schema.get("oneOf").and_then(Value::as_array) {
        if alternatives.is_empty() {
            bail!("oneOf must have at least one alternative");
        }
        let idx = pick_one_of_index(ctx, schema, value_path);
        let branch = alternatives
            .get(idx)
            .ok_or_else(|| anyhow!("oneOf index out of range"))?;
        return resolve_schema_in_context(ctx, branch, value_path);
    }
    Ok(schema)
}

pub fn resolve_schema_at_path_with<'a>(
    path: &[String],
    ctx: ResolveCtx<'a>,
) -> Result<&'a Value> {
    let mut schema: &'a Value = ctx.root;
    for (i, segment) in path.iter().enumerate() {
        let prefix: Vec<String> = path[..i].to_owned();
        schema = resolve_schema_in_context(ctx, schema, &prefix)?;
        if let Ok(index) = segment.parse::<usize>() {
            let prefix_schema = schema
                .get("prefixItems")
                .and_then(Value::as_array)
                .and_then(|items| items.get(index));
            if let Some(prefix_schema) = prefix_schema {
                schema = prefix_schema;
                continue;
            }
            schema = schema
                .get("items")
                .ok_or_else(|| anyhow!("array schema missing items at {}", path.join(".")))?;
        } else {
            schema = schema
                .get("properties")
                .and_then(Value::as_object)
                .and_then(|properties| properties.get(segment))
                .ok_or_else(|| anyhow!("schema path not found: {}", path.join(".")))?;
        }
    }
    resolve_schema_in_context(ctx, schema, path)
}

/// Resolves `$ref` and `oneOf` (first branch when no instance / choices).
pub fn resolve_schema_at_path<'a>(root: &'a Value, path: &[String]) -> Result<&'a Value> {
    resolve_schema_at_path_with(
        path,
        ResolveCtx {
            root,
            instance: None,
            choices: None,
        },
    )
}

fn default_value_for_schema_ctx(
    ctx: &ResolveCtx<'_>,
    schema: &Value,
    value_path: &[String],
) -> Result<Value> {
    let schema = resolve_schema_in_context(*ctx, schema, value_path)?;

    if let Some(value) = schema.get("const") {
        return Ok(value.clone());
    }
    if let Some(value) = schema.get("default") {
        return Ok(value.clone());
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if let Some(first) = values.first() {
            return Ok(first.clone());
        }
    }

    match schema_type_name(schema) {
        Some("object") => {
            let mut object = Map::new();
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for (key, property_schema) in properties {
                    let mut child_path = value_path.to_vec();
                    child_path.push(key.clone());
                    object.insert(
                        key.clone(),
                        default_value_for_schema_ctx(ctx, property_schema, &child_path)?,
                    );
                }
            }
            Ok(Value::Object(object))
        }
        None if schema.get("properties").is_some() => {
            let mut object = Map::new();
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for (key, property_schema) in properties {
                    let mut child_path = value_path.to_vec();
                    child_path.push(key.clone());
                    object.insert(
                        key.clone(),
                        default_value_for_schema_ctx(ctx, property_schema, &child_path)?,
                    );
                }
            }
            Ok(Value::Object(object))
        }
        Some("array") => default_array_value_ctx(ctx, schema, value_path),
        Some("string") => Ok(Value::String(default_string_value(schema))),
        Some("integer") => Ok(Value::Number(Number::from(default_integer_value(schema)))),
        Some("number") => Ok(json!(default_number_value(schema))),
        Some("boolean") => Ok(Value::Bool(false)),
        Some("null") => Ok(Value::Null),
        Some(other) => bail!("unsupported schema type: {other}"),
        None => bail!("schema type is missing"),
    }
}

fn default_string_value(schema: &Value) -> String {
    let min_length = schema.get("minLength").and_then(Value::as_u64).unwrap_or(0) as usize;
    if min_length == 0 {
        String::new()
    } else {
        "x".repeat(min_length)
    }
}

fn default_integer_value(schema: &Value) -> i64 {
    if let Some(exclusive_minimum) = schema.get("exclusiveMinimum").and_then(Value::as_f64) {
        return exclusive_minimum.floor() as i64 + 1;
    }
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
        return minimum.ceil() as i64;
    }
    0
}

fn default_number_value(schema: &Value) -> f64 {
    if let Some(exclusive_minimum) = schema.get("exclusiveMinimum").and_then(Value::as_f64) {
        return exclusive_minimum + 1.0;
    }
    if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64) {
        return minimum;
    }
    0.0
}

fn default_array_value_ctx(
    ctx: &ResolveCtx<'_>,
    schema: &Value,
    prefix_path: &[String],
) -> Result<Value> {
    let mut values = Vec::new();

    if let Some(prefix_items) = schema.get("prefixItems").and_then(Value::as_array) {
        for (i, prefix_schema) in prefix_items.iter().enumerate() {
            let mut item_path_vec = prefix_path.to_vec();
            item_path_vec.push(i.to_string());
            values.push(default_value_for_schema_ctx(
                ctx,
                prefix_schema,
                &item_path_vec,
            )?);
        }
    }

    let min_items = schema.get("minItems").and_then(Value::as_u64).unwrap_or(0) as usize;
    let item_schema = schema.get("items");
    while values.len() < min_items {
        if let Some(item_schema) = item_schema {
            let i = values.len();
            values.push(default_value_for_schema_ctx(
                ctx,
                item_schema,
                &item_path(prefix_path, i),
            )?);
        } else {
            values.push(Value::Null);
        }
    }

    Ok(Value::Array(values))
}

pub fn set_scalar_value(
    current: &Value,
    path: &[String],
    schema_type: &SchemaType,
    edit_buffer: &str,
) -> Result<Value> {
    let parsed = match schema_type {
        SchemaType::String => Value::String(edit_buffer.to_owned()),
        SchemaType::Integer => {
            let parsed: i64 = edit_buffer.trim().parse()?;
            Value::Number(Number::from(parsed))
        }
        SchemaType::Number => {
            let parsed: f64 = edit_buffer.trim().parse()?;
            let number =
                Number::from_f64(parsed).ok_or_else(|| anyhow!("invalid floating point value"))?;
            Value::Number(number)
        }
        SchemaType::Boolean => match edit_buffer.trim() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            other => bail!("invalid boolean: {other}"),
        },
        SchemaType::Null => {
            if edit_buffer.trim() != "null" {
                bail!("null field accepts only `null`");
            }
            Value::Null
        }
    };

    let mut next = current.clone();
    set_value_at_path(&mut next, path, parsed)?;
    Ok(next)
}

fn one_of_branch_label(root: &Value, alt: &Value, index: usize) -> String {
    if let Ok(resolved) = resolve_ref_only(root, alt) {
        if let Some(title) = resolved.get("title").and_then(Value::as_str) {
            return title.to_owned();
        }
    }
    if let Some(reference) = alt.get("$ref").and_then(Value::as_str) {
        if let Some(seg) = reference.rsplit('/').next() {
            return seg.to_owned();
        }
    }
    format!("variant-{index}")
}

fn flatten_fields(
    ctx: &ResolveCtx<'_>,
    schema: &Value,
    value: &Value,
    path: &[String],
    required: bool,
    output: &mut Vec<FormField>,
) {
    let Ok(schema) = resolve_schema_in_context(*ctx, schema, path) else {
        return;
    };

    if matches!(schema_type_name(schema), Some("object")) || schema.get("properties").is_some() {
        let required_set: Vec<String> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (key, child_schema) in properties {
                let child_value = value.get(key).unwrap_or(&Value::Null);
                let mut next_path = path.to_vec();
                next_path.push(key.clone());
                let Ok(raw_child) = resolve_ref_only(ctx.root, child_schema) else {
                    continue;
                };
                if let Some(alternatives) = raw_child.get("oneOf").and_then(Value::as_array) {
                    if alternatives.is_empty() {
                        continue;
                    }
                    let idx = pick_one_of_index(*ctx, raw_child, &next_path);
                    let labels: Vec<String> = alternatives
                        .iter()
                        .enumerate()
                        .map(|(i, alt)| one_of_branch_label(ctx.root, alt, i))
                        .collect();
                    let display = labels.get(idx).cloned().unwrap_or_default();
                    let key_str = next_path.join(".");
                    output.push(FormField {
                        path: next_path.clone(),
                        key: format!("{key_str}.oneOf"),
                        label: format!("{key} (oneOf)"),
                        description: Some("h / l or type letter to switch variant".to_owned()),
                        schema_type: SchemaType::String,
                        enum_options: Some(labels.clone()),
                        multiline: false,
                        required,
                        edit_buffer: display,
                        kind: FormFieldKind::OneOfSelector {
                            branch_count: alternatives.len(),
                        },
                    });
                    let branch = &alternatives[idx];
                    flatten_fields(ctx, branch, child_value, &next_path, required, output);
                } else {
                    flatten_fields(
                        ctx,
                        child_schema,
                        child_value,
                        &next_path,
                        required_set.contains(key),
                        output,
                    );
                }
            }
        }
        return;
    }

    if matches!(schema_type_name(schema), Some("array")) {
        let items = value.as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            let key = path.join(".");
            let short_key = path.last().cloned().unwrap_or_else(|| "Array".to_owned());
            let label = schema
                .get("title")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or(short_key);
            output.push(FormField {
                path: path.to_vec(),
                key,
                label,
                description: field_description(schema),
                schema_type: SchemaType::String,
                enum_options: None,
                multiline: false,
                required,
                edit_buffer: String::new(),
                kind: FormFieldKind::ArrayPlaceholder,
            });
            return;
        }

        let prefix_items = schema.get("prefixItems").and_then(Value::as_array);
        for (index, item) in items.iter().enumerate() {
            let child_schema = prefix_items
                .and_then(|prefix_items| prefix_items.get(index))
                .or_else(|| schema.get("items"));
            let Some(child_schema) = child_schema else {
                continue;
            };
            let mut next_path = path.to_vec();
            next_path.push(index.to_string());
            flatten_fields(ctx, child_schema, item, &next_path, required, output);
        }
        return;
    }

    if let Some(schema_type) = parse_scalar_type(schema) {
        let key = path.join(".");
        let short_key = path.last().cloned().unwrap_or_default();
        let label = schema
            .get("title")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or(short_key);
        output.push(FormField {
            path: path.to_vec(),
            key,
            label,
            description: field_description(schema),
            schema_type,
            enum_options: enum_option_buffers(schema),
            multiline: is_multiline_field(schema),
            required,
            edit_buffer: scalar_to_buffer(value),
            kind: FormFieldKind::Scalar,
        });
    }
}

fn is_multiline_field(schema: &Value) -> bool {
    if schema_type_name(schema) != Some("string") {
        return false;
    }
    if schema.get("format").and_then(Value::as_str) == Some("textarea") {
        return true;
    }
    schema
        .get("default")
        .and_then(Value::as_str)
        .map(|text| text.contains('\n'))
        .unwrap_or(false)
}

fn scalar_to_buffer(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Null => "null".to_owned(),
        _ => value.to_string(),
    }
}

fn set_value_at_path(target: &mut Value, path: &[String], value: Value) -> Result<()> {
    if path.is_empty() {
        *target = value;
        return Ok(());
    }

    let mut current = target;
    for key in &path[..path.len() - 1] {
        if let Ok(index) = key.parse::<usize>() {
            current = current
                .as_array_mut()
                .and_then(|items| items.get_mut(index))
                .ok_or_else(|| anyhow!("array path not found: {index}"))?;
        } else {
            current = current
                .as_object_mut()
                .and_then(|object| object.get_mut(key))
                .ok_or_else(|| anyhow!("object path not found: {key}"))?;
        }
    }

    let last = path.last().expect("path is not empty");
    if let Ok(index) = last.parse::<usize>() {
        let items = current
            .as_array_mut()
            .ok_or_else(|| anyhow!("expected array at {}", path.join(".")))?;
        let slot = items
            .get_mut(index)
            .ok_or_else(|| anyhow!("array index not found: {index}"))?;
        *slot = value;
        return Ok(());
    }

    let object = current
        .as_object_mut()
        .ok_or_else(|| anyhow!("expected object at {}", path.join(".")))?;
    object.insert(last.clone(), value);
    Ok(())
}

fn push_value_at_path(target: &mut Value, path: &[String], value: Value) -> Result<()> {
    let array = value_at_path_mut(target, path)?
        .as_array_mut()
        .ok_or_else(|| anyhow!("expected array at {}", path.join(".")))?;
    array.push(value);
    Ok(())
}

fn remove_value_at_path(target: &mut Value, path: &[String], index: usize) -> Result<()> {
    let array = value_at_path_mut(target, path)?
        .as_array_mut()
        .ok_or_else(|| anyhow!("expected array at {}", path.join(".")))?;
    array.remove(index);
    Ok(())
}

/// Human-readable scalar at `path` for logs (same path rules as `set_scalar_value`).
pub fn json_scalar_display_at_path(instance: &Value, path: &[String]) -> String {
    value_at_path(instance, path)
        .map(scalar_to_buffer)
        .unwrap_or_else(|| "(absent)".to_owned())
}

fn value_at_path<'a>(target: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut current = target;
    for segment in path {
        if let Ok(index) = segment.parse::<usize>() {
            current = current.as_array()?.get(index)?;
        } else {
            current = current.as_object()?.get(segment)?;
        }
    }
    Some(current)
}

fn value_at_path_mut<'a>(target: &'a mut Value, path: &[String]) -> Result<&'a mut Value> {
    let mut current = target;
    for segment in path {
        if let Ok(index) = segment.parse::<usize>() {
            current = current
                .as_array_mut()
                .and_then(|items| items.get_mut(index))
                .ok_or_else(|| anyhow!("array path not found: {index}"))?;
        } else {
            current = current
                .as_object_mut()
                .and_then(|object| object.get_mut(segment))
                .ok_or_else(|| anyhow!("object path not found: {segment}"))?;
        }
    }
    Ok(current)
}

pub fn resolve_schema<'a>(root: &'a Value, schema: &'a Value) -> Result<&'a Value> {
    resolve_schema_in_context(
        ResolveCtx {
            root,
            instance: None,
            choices: None,
        },
        schema,
        &[],
    )
}

fn resolve_internal_ref<'a>(root: &'a Value, reference: &str) -> Result<&'a Value> {
    let Some(pointer) = reference.strip_prefix('#') else {
        bail!("only internal refs are supported: {reference}");
    };
    root.pointer(pointer)
        .ok_or_else(|| anyhow!("ref not found: {reference}"))
}

fn schema_type_name(schema: &Value) -> Option<&str> {
    schema.get("type").and_then(Value::as_str)
}

fn parse_scalar_type(schema: &Value) -> Option<SchemaType> {
    match schema_type_name(schema) {
        Some("string") => Some(SchemaType::String),
        Some("number") => Some(SchemaType::Number),
        Some("integer") => Some(SchemaType::Integer),
        Some("boolean") => Some(SchemaType::Boolean),
        Some("null") => Some(SchemaType::Null),
        _ => schema
            .get("enum")
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(schema_type_for_value),
    }
}

fn enum_option_buffers(schema: &Value) -> Option<Vec<String>> {
    if schema_type_name(schema) == Some("boolean") {
        return Some(vec!["true".to_owned(), "false".to_owned()]);
    }
    let values = schema.get("enum").and_then(Value::as_array)?;
    let mut options = Vec::with_capacity(values.len());
    for value in values {
        if schema_type_for_value(value).is_none() {
            return None;
        }
        options.push(scalar_to_buffer(value));
    }
    Some(options)
}

fn field_description(schema: &Value) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(description) = schema.get("description").and_then(Value::as_str) {
        parts.push(description.to_owned());
    }
    if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
        parts.push(format!("pattern: {pattern}"));
    }
    if let Some(min_length) = schema.get("minLength").and_then(Value::as_u64) {
        parts.push(format!("minLength: {min_length}"));
    }
    if let Some(max_length) = schema.get("maxLength").and_then(Value::as_u64) {
        parts.push(format!("maxLength: {max_length}"));
    }
    if let Some(format) = schema.get("format").and_then(Value::as_str) {
        parts.push(format!("format: {format}"));
    }
    if let Some(minimum) = schema.get("minimum") {
        parts.push(format!("minimum: {}", scalar_to_buffer(minimum)));
    }
    if let Some(maximum) = schema.get("maximum") {
        parts.push(format!("maximum: {}", scalar_to_buffer(maximum)));
    }
    if let Some(exclusive_minimum) = schema.get("exclusiveMinimum") {
        parts.push(format!(
            "exclusiveMinimum: {}",
            scalar_to_buffer(exclusive_minimum)
        ));
    }
    if let Some(exclusive_maximum) = schema.get("exclusiveMaximum") {
        parts.push(format!(
            "exclusiveMaximum: {}",
            scalar_to_buffer(exclusive_maximum)
        ));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" | "))
    }
}

fn schema_type_for_value(value: &Value) -> Option<SchemaType> {
    match value {
        Value::String(_) => Some(SchemaType::String),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(SchemaType::Integer),
        Value::Number(_) => Some(SchemaType::Number),
        Value::Bool(_) => Some(SchemaType::Boolean),
        Value::Null => Some(SchemaType::Null),
        Value::Array(_) | Value::Object(_) => None,
    }
}

#[allow(dead_code)]
fn _ordered_properties(schema: &Value) -> IndexMap<String, Value> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use crate::domain::validation::validate_document;

    use super::{
        FormFieldKind, SchemaType, build_form_fields, build_form_fields_with,
        default_value_for_schema, json_scalar_display_at_path, set_scalar_value,
    };

    #[test]
    fn json_scalar_display_at_path_formats_nested_values() {
        let v = json!({ "a": { "b": 42 }, "s": "hello" });
        assert_eq!(
            json_scalar_display_at_path(&v, &["a".into(), "b".into()]),
            "42"
        );
        assert_eq!(json_scalar_display_at_path(&v, &["s".into()]), "hello");
        assert_eq!(
            json_scalar_display_at_path(&v, &["missing".into()]),
            "(absent)"
        );
    }

    #[test]
    fn builds_defaults_with_internal_refs() {
        let schema = json!({
            "type": "object",
            "$defs": {
                "addr": {
                    "type": "object",
                    "properties": {
                        "city": { "type": "string", "default": "Tokyo" }
                    }
                }
            },
            "properties": {
                "address": { "$ref": "#/$defs/addr" }
            }
        });

        let value = default_value_for_schema(&schema, &schema).unwrap();
        assert_eq!(value, json!({"address": {"city": "Tokyo"}}));
    }

    #[test]
    fn one_of_first_ref_branch_defaults_fields_and_validates() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["electrode"],
            "properties": {
                "electrode": { "$ref": "#/$defs/electrode" }
            },
            "$defs": {
                "metal": {
                    "type": "object",
                    "required": ["kind"],
                    "properties": {
                        "kind": { "const": "metal" },
                        "thickness_mm": { "type": "number", "default": 0.5 }
                    }
                },
                "composite": {
                    "type": "object",
                    "required": ["kind"],
                    "properties": {
                        "kind": { "const": "composite" },
                        "layers": { "type": "integer", "default": 3 }
                    }
                },
                "electrode": {
                    "oneOf": [
                        { "$ref": "#/$defs/metal" },
                        { "$ref": "#/$defs/composite" }
                    ]
                }
            }
        });

        let value = default_value_for_schema(&schema, &schema).unwrap();
        assert_eq!(
            value,
            json!({
                "electrode": { "kind": "metal", "thickness_mm": 0.5 }
            })
        );

        let fields = build_form_fields(&schema, &schema, &value);
        let paths: Vec<_> = fields.iter().map(|f| f.path.join(".")).collect();
        assert!(
            paths.iter().any(|p| p == "electrode.thickness_mm"),
            "expected scalar field under oneOf branch; got {paths:?}"
        );

        let summary = validate_document(&schema, &value).unwrap();
        assert!(summary.is_valid, "{:?}", summary.errors);
    }

    #[test]
    fn material_schema_file_defaults_and_validates() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/material.json");
        let text = std::fs::read_to_string(path).expect("read schema/material.json");
        let schema: serde_json::Value = serde_json::from_str(&text).expect("parse material.json");
        let value = default_value_for_schema(&schema, &schema).expect("defaults for material.json");
        let summary = validate_document(&schema, &value).expect("compile schema");
        assert!(summary.is_valid, "{:?}", summary.errors);
        let fields = build_form_fields(&schema, &schema, &value);
        assert!(
            !fields.is_empty(),
            "expected form fields from material.json"
        );
    }

    #[test]
    fn wafer_mask_layout_schema_file_defaults_and_validates() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/wafer-mask-layout.schema.json");
        let text =
            std::fs::read_to_string(path).expect("read schema/wafer-mask-layout.schema.json");
        let schema: serde_json::Value =
            serde_json::from_str(&text).expect("parse wafer-mask-layout.schema.json");
        let value =
            default_value_for_schema(&schema, &schema).expect("defaults for wafer-mask-layout");
        let summary = validate_document(&schema, &value).expect("compile schema");
        assert!(summary.is_valid, "{:?}", summary.errors);
        assert_eq!(value["name"], "x");
        assert_eq!(value["openings"][0]["shape"]["radius_mm"], 1.0);
    }

    #[test]
    fn empty_arrays_are_exposed_as_array_placeholders() {
        let schema = json!({
            "type": "object",
            "properties": {
                "alignment_holes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" }
                        }
                    }
                }
            }
        });
        let value = default_value_for_schema(&schema, &schema).unwrap();
        let fields = build_form_fields(&schema, &schema, &value);

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].path, vec!["alignment_holes".to_owned()]);
        assert!(matches!(fields[0].kind, FormFieldKind::ArrayPlaceholder));
    }

    #[test]
    fn build_form_fields_infers_one_of_branch_from_const_discriminator() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "electrode": { "$ref": "#/$defs/electrode" }
            },
            "$defs": {
                "metal": {
                    "type": "object",
                    "required": ["kind"],
                    "properties": {
                        "kind": { "const": "metal" },
                        "thickness_mm": { "type": "number", "default": 0.5 }
                    }
                },
                "composite": {
                    "type": "object",
                    "required": ["kind"],
                    "properties": {
                        "kind": { "const": "composite" },
                        "layers": { "type": "integer", "default": 3 }
                    }
                },
                "electrode": {
                    "oneOf": [
                        { "$ref": "#/$defs/metal" },
                        { "$ref": "#/$defs/composite" }
                    ]
                }
            }
        });
        let value = json!({
            "electrode": { "kind": "composite", "layers": 3 }
        });
        let fields = build_form_fields_with(&schema, &schema, &value, None);
        assert!(
            fields.iter().any(|f| f.path == vec!["electrode".to_owned()]
                && matches!(f.kind, FormFieldKind::OneOfSelector { .. })
                && f.edit_buffer == "composite")
        );
        assert!(fields.iter().any(|f| f.key == "electrode.layers"));
    }

    #[test]
    fn empty_one_of_fails_default_generation() {
        let schema = json!({
            "type": "object",
            "$defs": {
                "bad": { "oneOf": [] }
            },
            "properties": {
                "x": { "$ref": "#/$defs/bad" }
            }
        });

        let err = default_value_for_schema(&schema, &schema).unwrap_err();
        assert!(
            err.to_string().contains("oneOf"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn builds_defaults_for_arrays_with_min_items() {
        let schema = json!({
            "type": "array",
            "minItems": 2,
            "items": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "default": "item" },
                    "enabled": { "type": "boolean", "default": true }
                }
            }
        });

        let value = default_value_for_schema(&schema, &schema).unwrap();
        assert_eq!(
            value,
            json!([
                {"name": "item", "enabled": true},
                {"name": "item", "enabled": true}
            ])
        );
    }

    #[test]
    fn builds_defaults_for_prefix_items() {
        let schema = json!({
            "type": "array",
            "prefixItems": [
                { "type": "string", "default": "api" },
                { "type": "integer", "default": 3 }
            ]
        });

        let value = default_value_for_schema(&schema, &schema).unwrap();
        assert_eq!(value, json!(["api", 3]));
    }

    #[test]
    fn flattens_scalar_fields() {
        let schema = json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string", "default": "x" },
                "enabled": { "type": "boolean", "default": true }
            }
        });
        let value = default_value_for_schema(&schema, &schema).unwrap();
        let fields = build_form_fields(&schema, &schema, &value);

        assert_eq!(fields.len(), 2);
        let name = fields.iter().find(|field| field.key == "name").unwrap();
        let enabled = fields.iter().find(|field| field.key == "enabled").unwrap();
        assert_eq!(name.schema_type, SchemaType::String);
        assert!(name.required);
        assert_eq!(enabled.edit_buffer, "true");
        assert_eq!(
            enabled.enum_options.as_ref().unwrap(),
            &vec!["true".to_owned(), "false".to_owned()]
        );
    }

    #[test]
    fn preserves_property_order_from_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "zeta": { "type": "string", "default": "z" },
                "alpha": { "type": "string", "default": "a" },
                "middle": { "type": "integer", "default": 1 }
            }
        });
        let value = default_value_for_schema(&schema, &schema).unwrap();
        let fields = build_form_fields(&schema, &schema, &value);

        let keys: Vec<_> = fields.iter().map(|field| field.key.as_str()).collect();
        assert_eq!(keys, vec!["zeta", "alpha", "middle"]);
    }

    #[test]
    fn flattens_enum_fields_as_selectable_scalars() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["draft", "live", "archived"],
                    "default": "live"
                }
            }
        });
        let value = default_value_for_schema(&schema, &schema).unwrap();
        let fields = build_form_fields(&schema, &schema, &value);

        let status = fields.iter().find(|field| field.key == "status").unwrap();
        assert_eq!(status.schema_type, SchemaType::String);
        assert_eq!(status.edit_buffer, "live");
        assert_eq!(
            status.enum_options.as_ref().unwrap(),
            &vec!["draft".to_owned(), "live".to_owned(), "archived".to_owned()]
        );
    }

    #[test]
    fn marks_textarea_strings_as_multiline() {
        let schema = json!({
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "format": "textarea",
                    "default": "hello"
                }
            }
        });
        let value = default_value_for_schema(&schema, &schema).unwrap();
        let fields = build_form_fields(&schema, &schema, &value);

        assert!(fields[0].multiline);
    }

    #[test]
    fn flattens_prefix_item_arrays_and_surfaces_pattern_hint() {
        let schema = json!({
            "type": "array",
            "prefixItems": [
                { "type": "string", "title": "Code", "pattern": "^[A-Z]{3}$", "default": "ABC" },
                { "type": "integer", "title": "Count", "default": 2 }
            ]
        });
        let value = default_value_for_schema(&schema, &schema).unwrap();
        let fields = build_form_fields(&schema, &schema, &value);

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].key, "0");
        assert!(
            fields[0]
                .description
                .as_ref()
                .unwrap()
                .contains("pattern: ^[A-Z]{3}$")
        );
        assert_eq!(fields[1].key, "1");
    }

    #[test]
    fn updates_scalar_field_by_path() {
        let current = json!({"profile": {"age": 30}});
        let next = set_scalar_value(
            &current,
            &["profile".to_owned(), "age".to_owned()],
            &SchemaType::Integer,
            "31",
        )
        .unwrap();

        assert_eq!(next, json!({"profile": {"age": 31}}));
    }
}
