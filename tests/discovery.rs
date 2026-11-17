use std::{fs, path::Path};

use cargo_recursive_clean::{CargoWorkspace, DEFAULT_IGNORED_PATTERNS, find_cargo_projects};

fn write_manifest(dir: &Path, contents: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("Cargo.toml"), contents).unwrap();
}

fn package(name: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n")
}

#[test]
fn finds_nested_projects() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_manifest(&root.join("a"), &package("a"));
    write_manifest(&root.join("nested/deep/b"), &package("b"));
    fs::create_dir_all(root.join("not_a_project")).unwrap();

    let projects = find_cargo_projects(root, DEFAULT_IGNORED_PATTERNS).unwrap();

    assert_eq!(projects.len(), 2);
    assert!(projects.contains_key(&root.join("a")));
    assert!(projects.contains_key(&root.join("nested/deep/b")));
}

#[test]
fn root_itself_can_be_a_project() {
    let tmp = tempfile::tempdir().unwrap();
    write_manifest(tmp.path(), &package("root"));

    let projects = find_cargo_projects(tmp.path(), DEFAULT_IGNORED_PATTERNS).unwrap();

    assert_eq!(projects.len(), 1);
    assert!(projects.contains_key(tmp.path()));
}

#[test]
fn default_patterns_skip_target_and_node_modules() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_manifest(&root.join("a"), &package("a"));
    write_manifest(&root.join("a/target/package/a-0.1.0"), &package("a"));
    write_manifest(&root.join("web/node_modules/pkg"), &package("pkg"));

    let projects = find_cargo_projects(root, DEFAULT_IGNORED_PATTERNS).unwrap();

    assert_eq!(projects.len(), 1);
    assert!(projects.contains_key(&root.join("a")));
}

#[test]
fn empty_patterns_ignore_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_manifest(&root.join("a"), &package("a"));
    write_manifest(&root.join("a/target/package/a-0.1.0"), &package("a"));

    let projects = find_cargo_projects(root, &[]).unwrap();

    assert_eq!(projects.len(), 2);
}

#[test]
fn custom_patterns_replace_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_manifest(&root.join("keep"), &package("keep"));
    write_manifest(&root.join("venv/skip"), &package("skip"));
    write_manifest(&root.join("keep/target/x"), &package("x"));

    let projects = find_cargo_projects(root, &["**/venv/**"]).unwrap();

    assert_eq!(projects.len(), 2);
    assert!(projects.contains_key(&root.join("keep")));
    assert!(projects.contains_key(&root.join("keep/target/x")));
}

#[test]
fn invalid_pattern_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(find_cargo_projects(tmp.path(), &["**/[unclosed"]).is_err());
}

#[test]
fn ignores_directories_named_cargo_toml() {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("weird/Cargo.toml")).unwrap();

    let projects = find_cargo_projects(tmp.path(), DEFAULT_IGNORED_PATTERNS).unwrap();

    assert!(projects.is_empty());
}

#[test]
fn links_workspace_members_to_parent() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let ws = root.join("ws");
    write_manifest(&ws, "[workspace]\nmembers = [\"crates/a\", \"b\"]\n");
    write_manifest(&ws.join("crates/a"), &package("a"));
    write_manifest(&ws.join("b"), &package("b"));
    write_manifest(&root.join("standalone"), &package("standalone"));

    let projects = find_cargo_projects(root, DEFAULT_IGNORED_PATTERNS).unwrap();
    let workspace = |p: &Path| projects[p].workspace.borrow().clone();

    assert_eq!(projects.len(), 4);
    assert_eq!(
        workspace(&ws),
        CargoWorkspace::WorkspaceMembers(vec![ws.join("b"), ws.join("crates/a")])
    );
    assert_eq!(
        workspace(&ws.join("crates/a")),
        CargoWorkspace::Parent(ws.clone())
    );
    assert_eq!(workspace(&ws.join("b")), CargoWorkspace::Parent(ws.clone()));
    assert_eq!(workspace(&root.join("standalone")), CargoWorkspace::None);
}

#[test]
fn package_with_empty_workspace_table_is_standalone() {
    let tmp = tempfile::tempdir().unwrap();
    write_manifest(
        &tmp.path().join("a"),
        &format!("{}\n[workspace]\n", package("a")),
    );

    let projects = find_cargo_projects(tmp.path(), DEFAULT_IGNORED_PATTERNS).unwrap();

    assert_eq!(
        *projects[&tmp.path().join("a")].workspace.borrow(),
        CargoWorkspace::None
    );
}

#[test]
fn glob_workspace_members_are_linked() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    write_manifest(
        &ws,
        "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"crates/legacy\"]\n",
    );
    write_manifest(&ws.join("crates/a"), &package("a"));
    write_manifest(&ws.join("crates/b"), &package("b"));
    write_manifest(&ws.join("crates/legacy"), &package("legacy"));
    write_manifest(&ws.join("tools/x"), &package("x"));

    let projects = find_cargo_projects(tmp.path(), DEFAULT_IGNORED_PATTERNS).unwrap();
    let workspace = |p: &Path| projects[p].workspace.borrow().clone();

    assert_eq!(
        workspace(&ws),
        CargoWorkspace::WorkspaceMembers(vec![ws.join("crates/a"), ws.join("crates/b")])
    );
    assert_eq!(
        workspace(&ws.join("crates/a")),
        CargoWorkspace::Parent(ws.clone())
    );
    assert_eq!(
        workspace(&ws.join("crates/b")),
        CargoWorkspace::Parent(ws.clone())
    );
    assert_eq!(workspace(&ws.join("crates/legacy")), CargoWorkspace::None);
    assert_eq!(workspace(&ws.join("tools/x")), CargoWorkspace::None);
}
