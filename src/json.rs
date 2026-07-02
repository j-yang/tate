//! `serde_json::Value` → [`TreeNode`] conversion (feature `json`).
//!
//! This is the free on-ramp to the tree algebra: anything representable as JSON
//! — and therefore anything reachable through serde (JSON, YAML, TOML, …, or
//! your own `#[derive(Serialize)]` type via `serde_json::to_value`) — becomes a
//! [`TreeNode`] you can [`tree_diff`](crate::tree::tree_diff),
//! [`tree_merge`](crate::tree::tree_merge), or [`patch`](crate::patch) with. No
//! format-parsing crate required.
//!
//! ## Mapping
//!
//! - **Object** → one child per key; the key becomes the child's `identity`
//!   (so siblings match by name, and reordering keys is not a change) and its
//!   `kind`.
//! - **Array** → one child per item, `kind = "[item]"`, no identity (positional
//!   matching — order matters).
//! - **Scalar** (string / number / bool / null) → the node's `text` (stringified)
//!   plus a single `value` attribute, so a scalar change surfaces as a
//!   `changed_attrs` entry as well as a text change.
//!
//! ```
//! use tate::json::from_json_value;
//! use tate::tree::tree_diff;
//! use serde_json::json;
//!
//! let a = from_json_value("root", &json!({"server": {"port": 8080}}));
//! let b = from_json_value("root", &json!({"server": {"port": 9090}}));
//! let d = tree_diff(&a, &b);
//! assert_eq!(d.changes[0].id, "port");
//! ```

use serde_json::Value;

use crate::tree::TreeNode;

/// Convert a [`serde_json::Value`] into a [`TreeNode`] rooted at `kind`.
///
/// `kind` is the root node's element type (commonly `"root"`). See the module
/// docs for the object/array/scalar mapping.
pub fn from_json_value(kind: &str, value: &Value) -> TreeNode {
    let mut node = TreeNode::new(kind);
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                let child = from_json_value(key, val);
                // Object keys carry identity = key so siblings match by name.
                // (A nested object already recurses with `kind = key`; give it
                // the identity too. Scalars/arrays keyed here likewise.)
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

/// Parse a JSON string and convert it to a [`TreeNode`] rooted at `"root"`.
///
/// A convenience wrapper over [`from_json_value`]; returns a parse error string
/// on invalid JSON.
pub fn from_json_str(json: &str) -> Result<TreeNode, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("parse: {e}"))?;
    Ok(from_json_value("root", &value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{tree_diff, ChangeKind};
    use serde_json::json;

    #[test]
    fn object_key_becomes_identity() {
        let t = from_json_value("root", &json!({"port": 8080, "host": "localhost"}));
        let ids: Vec<_> = t.children.iter().filter_map(|c| c.identity.clone()).collect();
        assert!(ids.contains(&"port".to_string()));
        assert!(ids.contains(&"host".to_string()));
    }

    #[test]
    fn scalar_change_is_modified() {
        let a = from_json_value("root", &json!({"port": 8080}));
        let b = from_json_value("root", &json!({"port": 9090}));
        let d = tree_diff(&a, &b);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(d.changes[0].kind, ChangeKind::Modified);
        assert_eq!(d.changes[0].id, "port");
    }

    #[test]
    fn key_reorder_is_not_a_change() {
        let a = from_json_value("root", &json!({"a": 1, "b": 2}));
        let b = from_json_value("root", &json!({"b": 2, "a": 1}));
        assert!(tree_diff(&a, &b).changes.is_empty());
    }

    #[test]
    fn added_and_removed_keys() {
        let a = from_json_value("root", &json!({"a": 1}));
        let b = from_json_value("root", &json!({"a": 1, "b": 2}));
        let d = tree_diff(&a, &b);
        assert!(d.changes.iter().any(|c| c.kind == ChangeKind::Added && c.id == "b"));
    }

    #[test]
    fn array_items_are_positional() {
        let a = from_json_value("root", &json!({"items": ["a", "b", "c"]}));
        let b = from_json_value("root", &json!({"items": ["a", "x", "c"]}));
        let d = tree_diff(&a, &b);
        assert!(d.changes.iter().any(|c| c.id == "items"));
    }

    #[test]
    fn from_str_roundtrips_to_same_tree() {
        let v = json!({"x": {"y": [1, 2]}});
        let from_val = from_json_value("root", &v);
        let from_str = from_json_str(&v.to_string()).unwrap();
        assert_eq!(from_val, from_str);
    }

    #[test]
    fn invalid_json_errors() {
        assert!(from_json_str("not json").is_err());
    }
}
