pub const BASIC: &str = r##"{
  "title": "Basic",
  "type": "object",
  "required": ["name", "enabled"],
  "properties": {
    "name": { "type": "string", "default": "example" },
    "enabled": { "type": "boolean", "default": true },
    "count": { "type": "integer", "default": 1 }
  }
}"##;

pub const PROFILE: &str = r##"{
  "title": "Profile",
  "type": "object",
  "$defs": {
    "address": {
      "type": "object",
      "properties": {
        "city": { "type": "string", "default": "Tokyo" },
        "zip": { "type": "string", "default": "100-0001" }
      }
    }
  },
  "properties": {
    "username": { "type": "string", "default": "ryo" },
    "age": { "type": "integer", "default": 30 },
    "address": { "$ref": "#/$defs/address" }
  }
}"##;

pub const DEPLOY: &str = r##"{
  "title": "Deploy",
  "type": "object",
  "properties": {
    "service": { "type": "string", "enum": ["api", "worker", "frontend"] },
    "replicas": { "type": "integer", "default": 2 },
    "region": { "const": "ap-northeast-1" },
    "env": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "key": { "type": "string" },
          "value": { "type": "string" }
        }
      }
    }
  }
}"##;

pub fn get_schema(name: &str) -> Option<&'static str> {
    match name {
        "sample/basic" => Some(BASIC),
        "sample/profile" => Some(PROFILE),
        "sample/deploy" => Some(DEPLOY),
        _ => None,
    }
}

pub fn names() -> &'static [&'static str] {
    &["sample/basic", "sample/profile", "sample/deploy"]
}
