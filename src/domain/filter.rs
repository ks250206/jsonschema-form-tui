use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct FilterOutcome {
    pub text: String,
    pub error: Option<String>,
}

impl Default for FilterOutcome {
    fn default() -> Self {
        Self {
            text: "null".to_owned(),
            error: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum FilterError {
    #[error("filter must start with '.'")]
    InvalidRoot,
    #[error("property not found: {0}")]
    MissingProperty(String),
    #[error("index out of bounds: {0}")]
    MissingIndex(usize),
    #[error("invalid token: {0}")]
    InvalidToken(String),
}

pub fn evaluate_filter(value: &Value, input: &str) -> FilterOutcome {
    match apply_filter(value, input) {
        Ok(found) => FilterOutcome {
            text: pretty_json(&found).unwrap_or_else(|_| "null".to_owned()),
            error: None,
        },
        Err(err) => FilterOutcome {
            text: pretty_json(value).unwrap_or_else(|_| "null".to_owned()),
            error: Some(err.to_string()),
        },
    }
}

pub fn pretty_json(value: &Value) -> Result<String, serde_json::Error> {
    let mut buffer = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
    value.serialize(&mut serializer)?;
    String::from_utf8(buffer).map_err(|err| {
        serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData, err))
    })
}

pub fn apply_filter(value: &Value, input: &str) -> Result<Value, FilterError> {
    let input = input.trim();
    if input == "." {
        return Ok(value.clone());
    }
    if !input.starts_with('.') {
        return Err(FilterError::InvalidRoot);
    }

    let mut current = value;
    let mut token = String::new();
    let chars: Vec<char> = input.chars().collect();
    let mut idx = 1;

    while idx < chars.len() {
        match chars[idx] {
            '.' => {
                if !token.is_empty() {
                    current = current
                        .get(&token)
                        .ok_or_else(|| FilterError::MissingProperty(token.clone()))?;
                    token.clear();
                }
                idx += 1;
            }
            '[' => {
                if !token.is_empty() {
                    current = current
                        .get(&token)
                        .ok_or_else(|| FilterError::MissingProperty(token.clone()))?;
                    token.clear();
                }
                idx += 1;
                let start = idx;
                while idx < chars.len() && chars[idx] != ']' {
                    idx += 1;
                }
                if idx >= chars.len() {
                    return Err(FilterError::InvalidToken(input.to_owned()));
                }
                let number: usize = chars[start..idx]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .map_err(|_| FilterError::InvalidToken(input.to_owned()))?;
                current = current
                    .get(number)
                    .ok_or(FilterError::MissingIndex(number))?;
                idx += 1;
            }
            c => {
                token.push(c);
                idx += 1;
            }
        }
    }

    if !token.is_empty() {
        current = current
            .get(&token)
            .ok_or_else(|| FilterError::MissingProperty(token.clone()))?;
    }

    Ok(current.clone())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{apply_filter, pretty_json};

    #[test]
    fn applies_nested_property_and_index_filter() {
        let value = json!({
            "service": {
                "ports": [8080, 8081]
            }
        });

        assert_eq!(
            apply_filter(&value, ".service.ports[1]").unwrap(),
            json!(8081)
        );
    }

    #[test]
    fn rejects_invalid_root() {
        let value = json!({"a": 1});
        let err = apply_filter(&value, "a").unwrap_err();
        assert_eq!(err.to_string(), "filter must start with '.'");
    }

    #[test]
    fn pretty_prints_with_four_space_indent() {
        let value = json!({"a": {"b": 1}});
        let pretty = pretty_json(&value).unwrap();
        assert!(pretty.contains("\n    \"a\""));
        assert!(pretty.contains("\n        \"b\""));
    }
}
