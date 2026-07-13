mod convert;
mod inline;
mod lcs;
mod lines;
mod unified;

use clap::{Parser, Subcommand};
use serde_json::Value;
use std::collections::HashMap;
use tate::patch::{self, merge_sections, Patch, SectionConflict, SectionConflictKind};
use tate::tree::{self, ChangeKind, TreeNode};

#[derive(Parser)]
#[command(
    name = "tate",
    version,
    about = "Structural diff and sheaf-pushout merge for JSON, YAML, TOML, and text"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Diff two files (auto-detect: JSON, YAML, TOML, or text)
    Diff {
        a: String,
        b: String,
        #[arg(long)]
        json: bool,
    },
    /// Diff two JSON trees from stdin: {"base":{}, "other":{}}
    TreeDiff,
    /// 3-way sheaf merge of JSON trees from stdin:
    /// {"base":{}, "ours":{}, "theirs":{}} → {"merged":{}, "conflicts":[]}
    TreeMerge,
    /// Lossless patch algebra (diff/apply/invert/compose)
    Patch {
        #[command(subcommand)]
        action: PatchCmd,
    },
    /// Git external diff driver (called by git via diff.<name>.command)
    GitDiff {
        #[arg(allow_hyphen_values = true, num_args = 7)]
        args: Vec<String>,
    },
    /// Git merge driver: base ours theirs (writes result to ours; exits 1 on conflict)
    GitMerge {
        base: String,
        ours: String,
        theirs: String,
    },
}

#[derive(Subcommand)]
enum PatchCmd {
    /// Generate a lossless patch between two files
    Diff { a: String, b: String },
    /// Apply a patch file to an input file, writing the result to stdout
    Apply { patch: String, input: String },
    /// Invert a patch (swap old/new in every edit)
    Invert { patch: String },
    /// Compose two patches sequentially (first then second)
    Compose { first: String, second: String },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Diff { a, b, json } => cmd_diff(&a, &b, json),
        Command::TreeDiff => cmd_tree_diff(),
        Command::TreeMerge => cmd_tree_merge(),
        Command::Patch { action } => match action {
            PatchCmd::Diff { a, b } => cmd_patch_diff(&a, &b),
            PatchCmd::Apply { patch, input } => cmd_patch_apply(&patch, &input),
            PatchCmd::Invert { patch } => cmd_patch_invert(&patch),
            PatchCmd::Compose { first, second } => cmd_patch_compose(&first, &second),
        },
        Command::GitDiff { args } => cmd_git_diff(&args),
        Command::GitMerge { base, ours, theirs } => cmd_git_merge(&base, &ours, &theirs),
    };
    if let Err(e) = result {
        eprintln!("tate: {e}");
        std::process::exit(1);
    }
}

// ─── diff ────────────────────────────────────────────────────────────────────

fn cmd_diff(a: &str, b: &str, json: bool) -> Result<(), String> {
    let fmt = convert::detect(a);
    match fmt {
        convert::Format::Text => cmd_text_diff(a, b),
        _ => {
            let ta = convert::file_to_tree(a, fmt)?;
            let tb = convert::file_to_tree(b, fmt)?;
            let diff = tree::tree_diff(&ta, &tb);
            if json {
                println!("{}", serde_json::to_string_pretty(&diff).map_err(|e| e.to_string())?);
            } else {
                print_structural_diff(a, b, &diff);
            }
            Ok(())
        }
    }
}

fn print_structural_diff(a: &str, b: &str, diff: &tree::TreeDiff) {
    println!("diff --tate {} {}", a, b);
    if diff.changes.is_empty() {
        println!("(no changes)");
        return;
    }
    for c in &diff.changes {
        let kind = match c.kind {
            ChangeKind::Added => "added",
            ChangeKind::Removed => "removed",
            ChangeKind::Modified => "modified",
        };
        let path = if c.path.is_empty() {
            c.id.clone()
        } else {
            c.path.last().cloned().unwrap_or_default()
        };
        match c.kind {
            ChangeKind::Added | ChangeKind::Removed => {
                println!("  {kind:10} {path}");
            }
            ChangeKind::Modified => {
                for attr in &c.changed_attrs {
                    if attr.name == "value" {
                        println!("  {kind:10} {path:30} {} → {}", attr.old, attr.new);
                    } else {
                        println!("  {kind:10} {path}.{}: {} → {}", attr.name, attr.old, attr.new);
                    }
                }
                if let Some((old, new)) = &c.changed_text {
                    if c.changed_attrs.is_empty() {
                        println!("  {kind:10} {path:30} {old} → {new}");
                    }
                }
            }
        }
    }
}

fn cmd_text_diff(a: &str, b: &str) -> Result<(), String> {
    let lines_a = read_lines(a)?;
    let lines_b = read_lines(b)?;
    let ops = lines::diff(&lines_a, &lines_b);
    let paired = inline::pair_replacements(ops, inline::DEFAULT_SIMILARITY);
    let unified = unified::to_unified(&paired, 3);
    if unified.is_empty() {
        println!("diff --tate {} {}", a, b);
        println!("(no changes)");
    } else {
        print!("diff --tate {} {}\n{}", a, b, unified);
    }
    Ok(())
}

fn read_lines(path: &str) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let normalized = text.replace("\r\n", "\n");
    let mut lines: Vec<String> = normalized.split('\n').map(String::from).collect();
    if lines.last().map(|s| s.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    Ok(lines)
}

// ─── tree-diff / tree-merge (stdin JSON API) ─────────────────────────────────

fn read_stdin_json() -> Result<serde_json::Value, String> {
    let mut input = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
        .map_err(|e| format!("read stdin: {e}"))?;
    if input.is_empty() {
        return Err("no input on stdin".into());
    }
    serde_json::from_str(&input).map_err(|e| format!("parse JSON: {e}"))
}

fn get_field<'a>(input: &'a serde_json::Value, key: &str) -> Result<&'a serde_json::Value, String> {
    input.get(key).ok_or_else(|| format!("missing '{key}' in input JSON"))
}

fn value_to_tree(v: &serde_json::Value) -> TreeNode {
    convert::from_json_value("root", v)
}

fn cmd_tree_diff() -> Result<(), String> {
    let input = read_stdin_json()?;
    let base = value_to_tree(get_field(&input, "base")?);
    let other = value_to_tree(get_field(&input, "other")?);
    let diff = tree::tree_diff(&base, &other);
    println!("{}", serde_json::to_string_pretty(&diff).map_err(|e| e.to_string())?);
    Ok(())
}

fn cmd_tree_merge() -> Result<(), String> {
    let input = read_stdin_json()?;
    let base = value_to_tree(get_field(&input, "base")?);
    let ours = value_to_tree(get_field(&input, "ours")?);
    let theirs = value_to_tree(get_field(&input, "theirs")?);
    let r = merge_sections(&base.to_section(), &ours.to_section(), &theirs.to_section());
    let merged = r.merged.to_tree().unwrap_or_else(|| base.clone());
    let merged_json = convert::tree_to_json_value(&merged);
    let conflicts: Vec<serde_json::Value> = r.conflicts.iter().map(conflict_to_json).collect();
    let out = serde_json::json!({ "merged": merged_json, "conflicts": conflicts });
    println!("{}", serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?);
    Ok(())
}

fn conflict_to_json(c: &SectionConflict) -> serde_json::Value {
    let kind = match c.kind {
        SectionConflictKind::Field => "Field",
        SectionConflictKind::Dangling => "Dangling",
    };
    serde_json::json!({
        "kind": kind,
        "identity": c.identity,
        "missing_parent": c.missing_parent,
    })
}

// ─── patch algebra ───────────────────────────────────────────────────────────

fn read_patch(path: &str) -> Result<Patch, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    serde_json::from_str(&s).map_err(|e| format!("parse patch: {e}"))
}

fn write_patch(p: &Patch) -> Result<String, String> {
    serde_json::to_string_pretty(p).map_err(|e| format!("serialize patch: {e}"))
}

fn tree_to_string(tree: &TreeNode, fmt: convert::Format) -> Result<String, String> {
    match fmt {
        convert::Format::Json => Ok(convert::tree_to_json_pretty(tree)),
        convert::Format::Yaml => {
            let jv = convert::tree_to_json_value(tree);
            serde_yaml::to_string(&jv).map_err(|e| format!("serialize YAML: {e}"))
        }
        convert::Format::Toml => {
            let jv = convert::tree_to_json_value(tree);
            toml::to_string(&jv).map_err(|e| format!("serialize TOML: {e}"))
        }
        convert::Format::Text => Err("text format has no tree serialization".into()),
    }
}

fn cmd_patch_diff(a: &str, b: &str) -> Result<(), String> {
    let fmt = convert::detect(a);
    let ta = convert::file_to_tree(a, fmt)?;
    let tb = convert::file_to_tree(b, fmt)?;
    let p = patch::diff(&ta, &tb);
    println!("{}", write_patch(&p)?);
    Ok(())
}

fn cmd_patch_apply(patch_path: &str, input_path: &str) -> Result<(), String> {
    let p = read_patch(patch_path)?;
    let fmt = convert::detect(input_path);
    let tree = convert::file_to_tree(input_path, fmt)?;
    let result = patch::apply(&p, &tree).map_err(|e| format!("apply failed: {e}"))?;
    println!("{}", tree_to_string(&result, fmt)?);
    Ok(())
}

fn cmd_patch_invert(patch_path: &str) -> Result<(), String> {
    let p = read_patch(patch_path)?;
    let inv = patch::invert(&p);
    println!("{}", write_patch(&inv)?);
    Ok(())
}

fn cmd_patch_compose(first: &str, second: &str) -> Result<(), String> {
    let p1 = read_patch(first)?;
    let p2 = read_patch(second)?;
    let composed = patch::compose(&p1, &p2);
    println!("{}", write_patch(&composed)?);
    Ok(())
}

// ─── git integration ─────────────────────────────────────────────────────────

fn cmd_git_diff(args: &[String]) -> Result<(), String> {
    if args.len() < 7 {
        return Err("git-diff needs 7 arguments (path old-file old-hex old-mode new-file new-hex new-mode)".into());
    }
    let path = &args[0];
    let old_file = &args[1];
    let new_file = &args[4];
    let fmt = convert::detect(path);

    match fmt {
        convert::Format::Text => {
            let lines_a = read_lines(old_file)?;
            let lines_b = read_lines(new_file)?;
            let ops = lines::diff(&lines_a, &lines_b);
            let paired = inline::pair_replacements(ops, inline::DEFAULT_SIMILARITY);
            let unified = unified::to_unified(&paired, 3);
            print!("diff --tate a/{path} b/{path}\n{unified}");
        }
        _ => {
            let ta = convert::file_to_tree(old_file, fmt)?;
            let tb = convert::file_to_tree(new_file, fmt)?;
            let diff = tree::tree_diff(&ta, &tb);
            print!("diff --tate a/{path} b/{path}\n");
            if diff.changes.is_empty() {
                println!("(no changes)");
            } else {
                for c in &diff.changes {
                    let kind = match c.kind {
                        ChangeKind::Added => "+",
                        ChangeKind::Removed => "-",
                        ChangeKind::Modified => "~",
                    };
                    let loc = c.path.last().cloned().unwrap_or_else(|| c.id.clone());
                    println!("  {kind} {loc}");
                }
            }
        }
    }
    Ok(())
}

/// JSON scalar value of a leaf node, mirroring convert's leaf logic.
fn node_scalar_value(n: &tate::section::Node) -> Value {
    use serde_json::Number;
    if let Some(v) = n.attrs.iter().find(|(k, _)| k == "value").map(|(_, v)| v.as_str()) {
        return match v {
            "null" => Value::Null,
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => match v.parse::<f64>() {
                Ok(num) if num.fract() == 0.0 && num.abs() < 1e15 => {
                    Value::Number(Number::from(num as i64))
                }
                Ok(num) => Number::from_f64(num).map(Value::Number).unwrap_or_else(|| Value::String(v.into())),
                Err(_) => Value::String(v.into()),
            },
        };
    }
    Value::String(n.text.clone())
}

/// Pretty-print the merged tree as JSON, injecting git-native conflict markers
/// (`<<<<<<<` / `=======` / `>>>>>>>`) at Field-conflicted leaves. The output is
/// intentionally not valid JSON while conflicted — exactly like git's text merge;
/// the user resolves the markers, then `git add`s. Non-leaf and Dangling conflicts
/// are left best-effort and reported on stderr.
fn merge_marked_json(merged: &TreeNode, conflicts: &HashMap<String, (Value, Value)>) -> String {
    fn emit(node: &TreeNode, conflicts: &HashMap<String, (Value, Value)>, level: usize) -> String {
        let pad = "  ".repeat(level);
        if node.children.is_empty() {
            return convert::tree_to_json_value(node).to_string();
        }
        let is_array = node.children.iter().all(|c| c.kind == "[item]");
        let pad2 = "  ".repeat(level + 1);
        let parts: Vec<String> = node.children.iter().map(|c| {
            let eff_key = c.identity.clone().unwrap_or_else(|| c.kind.clone());
            let marker = conflicts.get(&eff_key)
                .filter(|_| c.children.is_empty())
                .map(|(o, t)| {
                    if is_array {
                        format!("<<<<<<< ours\n{pad2}{o}\n=======\n{pad2}{t}\n>>>>>>> theirs")
                    } else {
                        format!("<<<<<<< ours\n{pad2}\"{eff_key}\": {o}\n=======\n{pad2}\"{eff_key}\": {t}\n>>>>>>> theirs")
                    }
                });
            if let Some(block) = marker {
                return block;
            }
            let body = emit(c, conflicts, level + 1);
            if is_array {
                format!("{pad2}{body}")
            } else {
                format!("{pad2}\"{eff_key}\": {body}")
            }
        }).collect();
        let (open, close) = if is_array { ("[", "]") } else { ("{", "}") };
        if parts.is_empty() {
            return format!("{open}{close}");
        }
        format!("{open}\n{}\n{pad}{close}", parts.join(",\n"))
    }
    emit(merged, conflicts, 0)
}

fn cmd_git_merge(base: &str, ours: &str, theirs: &str) -> Result<(), String> {
    let fmt = convert::detect(ours);
    if fmt == convert::Format::Text {
        return Err("tate git-merge does not support text format — use git's built-in merge".into());
    }
    let tb = convert::file_to_tree(base, fmt)?;
    let to = convert::file_to_tree(ours, fmt)?;
    let tt = convert::file_to_tree(theirs, fmt)?;
    let result = merge_sections(&tb.to_section(), &to.to_section(), &tt.to_section());

    let merged_tree = result
        .merged
        .to_tree()
        .ok_or_else(|| "merge produced an empty tree".to_string())?;

    let has_conflicts = !result.conflicts.is_empty();
    let output = if has_conflicts && fmt == convert::Format::Json {
        let mut sides: HashMap<String, (Value, Value)> = HashMap::new();
        for c in &result.conflicts {
            if c.kind == SectionConflictKind::Field {
                if let (Some(o), Some(t)) = (&c.ours, &c.theirs) {
                    sides.insert(c.identity.clone(), (node_scalar_value(o), node_scalar_value(t)));
                }
            }
        }
        merge_marked_json(&merged_tree, &sides)
    } else {
        tree_to_string(&merged_tree, fmt)?
    };
    std::fs::write(ours, &output).map_err(|e| format!("write {ours}: {e}"))?;

    if result.conflicts.is_empty() {
        Ok(())
    } else {
        for c in &result.conflicts {
            match c.kind {
                SectionConflictKind::Dangling => {
                    eprintln!(
                        "CONFLICT (Dangling): {} — parent {} absent, node dropped",
                        c.identity,
                        c.missing_parent.as_deref().unwrap_or("?"),
                    );
                }
                SectionConflictKind::Field => {
                    eprintln!("CONFLICT (Field): {}", c.identity);
                }
            }
        }
        std::process::exit(1);
    }
}
