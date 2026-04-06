use anyhow::{Result, anyhow, bail};
use indexmap::IndexMap;
use serde_json::{Map, Number, Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaType {
    String,
    Number,
    Integer,
    Boolean,
    Null,
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
}

pub fn default_value_for_schema(root: &Value, schema: &Value) -> Result<Value> {
    let schema = resolve_schema(root, schema)?;

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
                    object.insert(
                        key.clone(),
                        default_value_for_schema(root, property_schema)?,
                    );
                }
            }
            Ok(Value::Object(object))
        }
        None if schema.get("properties").is_some() => {
            let mut object = Map::new();
            if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
                for (key, property_schema) in properties {
                    object.insert(
                        key.clone(),
                        default_value_for_schema(root, property_schema)?,
                    );
                }
            }
            Ok(Value::Object(object))
        }
        Some("array") => default_array_value(root, schema),
        Some("string") => Ok(Value::String(String::new())),
        Some("integer") => Ok(Value::Number(Number::from(0))),
        Some("number") => Ok(json!(0.0)),
        Some("boolean") => Ok(Value::Bool(false)),
        Some("null") => Ok(Value::Null),
        Some(other) => bail!("unsupported schema type: {other}"),
        None => bail!("schema type is missing"),
    }
}

pub fn build_form_fields(root: &Value, schema: &Value, value: &Value) -> Vec<FormField> {
    let mut fields = Vec::new();
    flatten_fields(root, schema, value, &[], false, &mut fields);
    fields
}

pub fn append_array_item(current: &Value, root: &Value, array_path: &[String]) -> Result<Value> {
    let array_schema = resolve_schema_at_path(root, array_path)?;
    let array_schema = resolve_schema(root, array_schema)?;
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
                default_value_for_schema(root, prefix_schema)?
            } else if let Some(items_schema) = array_schema.get("items") {
                default_value_for_schema(root, items_schema)?
            } else {
                bail!(
                    "array does not allow additional items: {}",
                    array_path.join(".")
                );
            }
        } else if let Some(items_schema) = array_schema.get("items") {
            default_value_for_schema(root, items_schema)?
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
) -> Result<Value> {
    let array_schema = resolve_schema_at_path(root, array_path)?;
    let array_schema = resolve_schema(root, array_schema)?;
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

pub fn resolve_schema_at_path<'a>(root: &'a Value, path: &[String]) -> Result<&'a Value> {
    let mut schema = root;
    for segment in path {
        schema = resolve_schema(root, schema)?;
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
    resolve_schema(root, schema)
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

fn flatten_fields(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &[String],
    required: bool,
    output: &mut Vec<FormField>,
) {
    let Ok(schema) = resolve_schema(root, schema) else {
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
                flatten_fields(
                    root,
                    child_schema,
                    child_value,
                    &next_path,
                    required_set.contains(key),
                    output,
                );
            }
        }
        return;
    }

    if matches!(schema_type_name(schema), Some("array")) {
        if let Some(items) = value.as_array() {
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
                flatten_fields(root, child_schema, item, &next_path, required, output);
            }
        }
        return;
    }

    if let Some(schema_type) = parse_scalar_type(schema) {
        let key = path.join(".");
        let label = schema
            .get("title")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| key.clone());
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

fn default_array_value(root: &Value, schema: &Value) -> Result<Value> {
    let mut values = Vec::new();

    if let Some(prefix_items) = schema.get("prefixItems").and_then(Value::as_array) {
        for prefix_schema in prefix_items {
            values.push(default_value_for_schema(root, prefix_schema)?);
        }
    }

    let min_items = schema.get("minItems").and_then(Value::as_u64).unwrap_or(0) as usize;
    let item_schema = schema.get("items");
    while values.len() < min_items {
        if let Some(item_schema) = item_schema {
            values.push(default_value_for_schema(root, item_schema)?);
        } else {
            values.push(Value::Null);
        }
    }

    Ok(Value::Array(values))
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

fn resolve_schema<'a>(root: &'a Value, schema: &'a Value) -> Result<&'a Value> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return resolve_internal_ref(root, reference);
    }
    Ok(schema)
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
    use serde_json::json;

    use super::{SchemaType, build_form_fields, default_value_for_schema, set_scalar_value};

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
