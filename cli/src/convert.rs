use serde_json::Value;
use std::path::Path;
use tate::tree::TreeNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Yaml,
    Toml,
    Text,
}

pub fn detect(path: &str) -> Format {
    match ext_of(path).as_deref() {
        Some("json") => Format::Json,
        Some("yaml") | Some("yml") => Format::Yaml,
        Some("toml") => Format::Toml,
        _ => Format::Text,
    }
}

fn ext_of(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .filter(|e| !e.is_empty())
}

pub fn from_json_value(kind: &str, value: &Value) -> TreeNode {
    let mut node = TreeNode::new(kind);
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                let child = from_json_value(key, val);
                let child = if child.identity.is_none() && !child.children.is_empty() {
                    TreeNode { identity: Some(key.clone()), ..child }
                } else {
                    child.with_identity(key.clone())
                };
                node = node.with_child(child);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                node = node.with_child(from_json_value("[item]", item));
            }
        }
        Value::String(s) => {
            node = node.with_text(s.clone()).with_attr("value", s.clone());
        }
        Value::Number(n) => {
            let s = n.to_string();
            node = node.with_text(s.clone()).with_attr("value", s);
        }
        Value::Bool(b) => {
            let s = b.to_string();
            node = node.with_text(s.clone()).with_attr("value", s);
        }
        Value::Null => {
            node = node.with_attr("value", "null");
        }
    }
    node
}

pub fn file_to_tree(path: &str, fmt: Format) -> Result<TreeNode, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("read {path}: {e}"))?;
    str_to_tree(&content, fmt)
}

pub fn str_to_tree(content: &str, fmt: Format) -> Result<TreeNode, String> {
    match fmt {
        Format::Json => {
            let v: Value = serde_json::from_str(content).map_err(|e| format!("parse JSON: {e}"))?;
            Ok(from_json_value("root", &v))
        }
        Format::Yaml => {
            let v: Value = serde_yaml::from_str(content).map_err(|e| format!("parse YAML: {e}"))?;
            Ok(from_json_value("root", &v))
        }
        Format::Toml => {
            let v: toml::Value = toml::from_str(content).map_err(|e| format!("parse TOML: {e}"))?;
            let jv = serde_json::to_value(&v).map_err(|e| format!("convert TOML: {e}"))?;
            Ok(from_json_value("root", &jv))
        }
        Format::Text => Err("text format has no tree representation".into()),
    }
}

pub fn tree_to_json_value(node: &TreeNode) -> Value {
    if node.children.is_empty() {
        if let Some(v) = node.attr("value") {
            if v == "null" {
                return Value::Null;
            }
            if v == "true" {
                return Value::Bool(true);
            }
            if v == "false" {
                return Value::Bool(false);
            }
            if let Ok(n) = v.parse::<f64>() {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    return Value::Number(serde_json::Number::from(n as i64));
                }
                if let Some(num) = serde_json::Number::from_f64(n) {
                    return Value::Number(num);
                }
            }
            return Value::String(v.into());
        }
        return Value::String(node.text.clone());
    }

    if node.children.iter().all(|c| c.kind == "[item]") {
        return Value::Array(node.children.iter().map(tree_to_json_value).collect());
    }

    let mut map = serde_json::Map::new();
    for child in &node.children {
        let key = child.identity.clone().unwrap_or_else(|| child.kind.clone());
        map.insert(key, tree_to_json_value(child));
    }
    Value::Object(map)
}

pub fn tree_to_json_pretty(node: &TreeNode) -> String {
    let value = tree_to_json_value(node);
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
}
