use std::fs;
use std::process::Command;

fn tate() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tate"))
}

fn tmp(name: &str) -> std::path::PathBuf {
    let id = std::process::id();
    std::env::temp_dir().join(format!("tate-test-{id}-{name}"))
}

fn write(name: &str, content: &str) -> std::path::PathBuf {
    let p = tmp(name);
    fs::write(&p, content).unwrap();
    p
}

fn run(args: &[&str]) -> (bool, String, String) {
    let output = tate().args(args).output().unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn json_diff_detects_change() {
    let a = write("a.json", r#"{"port": 8080, "host": "localhost"}"#);
    let b = write("b.json", r#"{"port": 9090, "host": "localhost"}"#);
    let (ok, stdout, _) = run(&["diff", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert!(ok);
    assert!(stdout.contains("port"));
    assert!(stdout.contains("8080"));
    assert!(stdout.contains("9090"));
}

#[test]
fn json_diff_no_changes() {
    let a = write("same_a.json", r#"{"x": 1}"#);
    let b = write("same_b.json", r#"{"x": 1}"#);
    let (ok, stdout, _) = run(&["diff", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert!(ok);
    assert!(stdout.contains("no changes"));
}

#[test]
fn yaml_diff_works() {
    let a = write("a.yaml", "port: 8080\nhost: localhost\n");
    let b = write("b.yaml", "port: 9090\nhost: localhost\n");
    let (ok, stdout, _) = run(&["diff", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert!(ok);
    assert!(stdout.contains("port"));
}

#[test]
fn text_diff_works() {
    let a = write("a.txt", "hello\nworld\n");
    let b = write("b.txt", "hello\nrust\n");
    let (ok, stdout, _) = run(&["diff", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert!(ok);
    assert!(stdout.contains("world") || stdout.contains("rust"));
}

#[test]
fn git_merge_clean_disjoint_json() {
    let base = write("base.json", r#"{"a": 1, "b": 2}"#);
    let ours = write("ours.json", r#"{"a": 9, "b": 2}"#);
    let theirs = write("theirs.json", r#"{"a": 1, "b": 8}"#);
    let (ok, _, _) = run(&[
        "git-merge", base.to_str().unwrap(), ours.to_str().unwrap(), theirs.to_str().unwrap(),
    ]);
    assert!(ok);
    let merged = fs::read_to_string(&ours).unwrap();
    assert!(merged.contains("9"), "ours' change to 'a' must survive: {merged}");
    assert!(merged.contains("8"), "theirs' change to 'b' must survive: {merged}");
}

#[test]
fn git_merge_conflict_same_key_json() {
    let base = write("conf_base.json", r#"{"port": 8080}"#);
    let ours = write("conf_ours.json", r#"{"port": 9090}"#);
    let theirs = write("conf_theirs.json", r#"{"port": 3000}"#);
    let (ok, _, stderr) = run(&[
        "git-merge", base.to_str().unwrap(), ours.to_str().unwrap(), theirs.to_str().unwrap(),
    ]);
    assert!(!ok, "conflicting merge must exit non-zero");
    assert!(stderr.contains("CONFLICT"));
}

#[test]
fn git_merge_clean_yaml() {
    let base = write("base.yaml", "a: 1\nb: 2\n");
    let ours = write("ours.yaml", "a: 9\nb: 2\n");
    let theirs = write("theirs.yaml", "a: 1\nb: 8\n");
    let (ok, _, _) = run(&[
        "git-merge", base.to_str().unwrap(), ours.to_str().unwrap(), theirs.to_str().unwrap(),
    ]);
    assert!(ok);
    let merged = fs::read_to_string(&ours).unwrap();
    assert!(merged.contains("a: 9"), "ours' change must survive: {merged}");
    assert!(merged.contains("b: 8"), "theirs' change must survive: {merged}");
}

#[test]
fn git_merge_toml_outputs_toml() {
    let base = write("base.toml", "a = 1\nb = 2\n");
    let ours = write("ours.toml", "a = 9\nb = 2\n");
    let theirs = write("theirs.toml", "a = 1\nb = 8\n");
    let (ok, _, _) = run(&[
        "git-merge", base.to_str().unwrap(), ours.to_str().unwrap(), theirs.to_str().unwrap(),
    ]);
    assert!(ok);
    let merged = fs::read_to_string(&ours).unwrap();
    assert!(merged.contains("a = 9"), "ours' change must survive: {merged}");
    assert!(merged.contains("b = 8"), "theirs' change must survive: {merged}");
    assert!(!merged.contains("{"), "TOML output must not be JSON: {merged}");
}

#[test]
fn patch_roundtrip_json() {
    let a = write("p_a.json", r#"{"port": 8080}"#);
    let b = write("p_b.json", r#"{"port": 9090}"#);
    let patch_file = tmp("patch.json");

    let (ok, stdout, _) = run(&["patch", "diff", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert!(ok);
    fs::write(&patch_file, &stdout).unwrap();

    let (ok, stdout, _) = run(&["patch", "apply", patch_file.to_str().unwrap(), a.to_str().unwrap()]);
    assert!(ok);
    assert!(stdout.contains("9090"), "applied patch must produce target value: {stdout}");
}

#[test]
fn patch_invert_roundtrip() {
    let a = write("inv_a.json", r#"{"x": 1}"#);
    let b = write("inv_b.json", r#"{"x": 2}"#);
    let patch_file = tmp("inv_patch.json");
    let inverted_file = tmp("inv_inverted.json");

    let (ok, stdout, _) = run(&["patch", "diff", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert!(ok);
    fs::write(&patch_file, &stdout).unwrap();

    let (ok, stdout, _) = run(&["patch", "invert", patch_file.to_str().unwrap()]);
    assert!(ok);
    fs::write(&inverted_file, &stdout).unwrap();

    let (ok, stdout, _) = run(&["patch", "apply", inverted_file.to_str().unwrap(), b.to_str().unwrap()]);
    assert!(ok);
    assert!(stdout.contains("1"), "inverted patch applied to b must recover a: {stdout}");
}

/// The sheaf merge's defining capability: a structural (Dangling) conflict —
/// move-child + delete-target — is reported, where the discrete merge is blind.
#[test]
fn tree_merge_reports_structural_conflict() {
    // base:   {"P": {"C": 1}, "Q": {}}     (C is a child of P; Q an empty container)
    // ours:   {"P": {"C": 1}}              (deletes Q)
    // theirs: {"P": {}, "Q": {"C": 1}}     (moves C under Q)
    // → C ends up present with parent Q absent → Dangling (and nothing else).
    let stdin = serde_json::json!({
        "base":   {"P": {"C": 1}, "Q": {}},
        "ours":   {"P": {"C": 1}},
        "theirs": {"P": {}, "Q": {"C": 1}},
    }).to_string();
    let mut child = tate()
        .args(["tree-merge"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "tree-merge should not fail on a conflict");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Dangling"), "a Dangling conflict must be reported: {stdout}");
}
