pub mod arg_parser;

use owo_colors::OwoColorize;
use rayon::iter::{IntoParallelRefIterator, ParallelExtend, ParallelIterator};
use std::{
    cell::RefCell,
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};
use wax::Program;

pub const DEFAULT_IGNORED_PATTERNS: &[&str] = &["**/node_modules/**", "**/target/**"];
pub const ASK_CONFIRMATION_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CargoProject {
    pub workspace: RefCell<CargoWorkspace>,
}

impl CargoProject {
    pub fn new(workspace: CargoWorkspace) -> Self {
        Self {
            workspace: RefCell::new(workspace),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CargoWorkspace {
    WorkspaceMembers(Vec<PathBuf>),
    Parent(PathBuf),
    None,
}

/// Returns the patterns to ignore while walking.
///
/// `None` means the user did not pass `--ignored-patterns`, so the defaults are used.
/// Blank entries are dropped, so `--ignored-patterns ''` disables ignoring entirely.
pub fn resolve_ignored_patterns(user_patterns: Option<&[String]>) -> Vec<&str> {
    match user_patterns {
        Some(patterns) => patterns
            .iter()
            .map(|pattern| pattern.trim())
            .filter(|pattern| !pattern.is_empty())
            .collect(),
        None => DEFAULT_IGNORED_PATTERNS.to_vec(),
    }
}

/// Builds the `cargo` arguments for a clean run.
pub fn cargo_clean_args(release_only: bool, doc_only: bool) -> Vec<&'static str> {
    let mut args = vec!["clean"];
    if release_only {
        args.push("--release");
    } else if doc_only {
        args.push("--doc");
    }
    args
}

/// The `[workspace]` section of a `Cargo.toml`, as written in the manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceManifest {
    /// Member paths or glob patterns (e.g. `crates/*`), relative to the workspace root.
    pub members: Vec<String>,
    /// Paths excluded from the workspace, relative to the workspace root.
    pub exclude: Vec<String>,
}

/// Parses the contents of a `Cargo.toml` and returns its `[workspace]` section, if any.
///
/// A manifest that fails to parse, or a `[workspace]` without `members`, yields `None`.
pub fn parse_workspace(manifest: &str) -> Option<WorkspaceManifest> {
    if !manifest.contains("[workspace]") {
        return None;
    }
    let parsed = toml::from_str::<toml::Table>(manifest).ok()?;
    let workspace = parsed.get("workspace")?;
    let strings = |key: &str| -> Vec<String> {
        workspace
            .get(key)
            .and_then(toml::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    workspace.get("members").and_then(toml::Value::as_array)?;
    Some(WorkspaceManifest {
        members: strings("members"),
        exclude: strings("exclude"),
    })
}

fn is_glob(member: &str) -> bool {
    member.contains(['*', '?', '[', '{'])
}

/// Resolves the members of the workspace rooted at `workspace_dir` to absolute paths.
///
/// Plain members are joined to `workspace_dir`. Glob members (e.g. `crates/*`) are
/// matched against `projects`, the directories known to contain a `Cargo.toml`.
/// Paths under any `exclude` entry, and the workspace root itself, are dropped.
pub fn resolve_workspace_members(
    workspace_dir: &Path,
    manifest: &WorkspaceManifest,
    projects: &[PathBuf],
) -> Vec<PathBuf> {
    let excluded: Vec<PathBuf> = manifest
        .exclude
        .iter()
        .map(|exclude| workspace_dir.join(exclude.trim_start_matches("./")))
        .collect();
    let mut resolved = Vec::new();

    for member in &manifest.members {
        let member = member.trim_start_matches("./").trim_end_matches('/');
        if !is_glob(member) {
            resolved.push(workspace_dir.join(member));
            continue;
        }
        let Ok(glob) = wax::Glob::new(member) else {
            continue;
        };
        resolved.extend(
            projects
                .iter()
                .filter(|project| {
                    project.strip_prefix(workspace_dir).is_ok_and(|relative| {
                        !relative.as_os_str().is_empty() && glob.is_match(relative)
                    })
                })
                .cloned(),
        );
    }

    // A workspace may list its own directory (`members = ["."]`). The root is the
    // workspace itself, not one of its members, so it is dropped.
    resolved.retain(|member| {
        member != workspace_dir && !excluded.iter().any(|exclude| member.starts_with(exclude))
    });
    resolved.sort();
    resolved.dedup();
    resolved
}

/// Ignore patterns split the same way `wax` does for `FileIterator::not`.
///
/// A directory matching an *exhaustive* pattern (e.g. `**/target/**`) is pruned without
/// being read. A `Cargo.toml` is skipped when it matches any pattern. Paths are matched
/// relative to the walk root.
///
/// Patterns of the form `**/<name>/**` with a plain `<name>` (the common case, including
/// the defaults) skip glob compilation and are matched by comparing the directory name.
struct IgnoreFilter<'t> {
    dir_names: Vec<&'t str>,
    exhaustive: Option<wax::Any<'t>>,
    nonexhaustive: Option<wax::Any<'t>>,
}

/// Compares path component names the way `wax` does: case-insensitively on Windows,
/// case-sensitively elsewhere.
fn path_names_equal(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Returns `<name>` if `pattern` is exactly `**/<name>/**` and `<name>` has no glob syntax.
fn literal_dir_pattern(pattern: &str) -> Option<&str> {
    let name = pattern.strip_prefix("**/")?.strip_suffix("/**")?;
    let is_plain = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    is_plain.then_some(name)
}

impl<'t> IgnoreFilter<'t> {
    fn new(patterns: &[&'t str]) -> Result<Self, wax::BuildError> {
        let mut dir_names = Vec::new();
        let mut globs = Vec::new();
        for pattern in patterns {
            match literal_dir_pattern(pattern) {
                Some(name) => dir_names.push(name),
                None => globs.push(wax::Glob::new(pattern)?),
            }
        }
        let (exhaustive, nonexhaustive): (Vec<_>, Vec<_>) = globs
            .into_iter()
            .partition(|glob| glob.is_exhaustive().is_always());
        let compile = |globs: Vec<wax::Glob<'t>>| {
            if globs.is_empty() {
                Ok(None)
            } else {
                wax::any(globs).map(Some)
            }
        };
        Ok(Self {
            dir_names,
            exhaustive: compile(exhaustive)?,
            nonexhaustive: compile(nonexhaustive)?,
        })
    }

    fn prunes_dir(&self, relative: &Path) -> bool {
        // Ancestors were already checked on the way down, so only the last component matters.
        let name_matches = !self.dir_names.is_empty()
            && relative
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| self.dir_names.iter().any(|dir| path_names_equal(dir, name)));
        name_matches
            || self
                .exhaustive
                .as_ref()
                .is_some_and(|any| any.is_match(relative))
    }

    fn skips_file(&self, relative: &Path) -> bool {
        self.prunes_dir(relative)
            || self
                .nonexhaustive
                .as_ref()
                .is_some_and(|any| any.is_match(relative))
    }
}

/// Walks `root` and returns every directory containing a `Cargo.toml`, skipping
/// paths matching `ignored_patterns`. Workspace members are marked with their parent.
///
/// Directories are read in parallel. Symbolic links are not followed. Directories that
/// cannot be read (e.g. permission denied) are reported on stderr and skipped.
pub fn find_cargo_projects(
    root: impl AsRef<Path>,
    ignored_patterns: &[&str],
) -> Result<HashMap<PathBuf, CargoProject>, Box<dyn std::error::Error>> {
    let root = root.as_ref();
    let filter = IgnoreFilter::new(ignored_patterns)?;

    let mut manifests = walk_dir(root, root, &filter);
    manifests.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
    let manifest_dirs: Vec<PathBuf> = manifests.iter().map(|(path, _)| path.clone()).collect();

    let cargo_projects = manifests
        .into_iter()
        .map(|(path, manifest)| {
            let workspace = match manifest {
                Some(manifest) => CargoWorkspace::WorkspaceMembers(resolve_workspace_members(
                    &path,
                    &manifest,
                    &manifest_dirs,
                )),
                None => CargoWorkspace::None,
            };
            (path, CargoProject::new(workspace))
        })
        .collect::<HashMap<PathBuf, CargoProject>>();

    find_cargo_workspaces(&cargo_projects);

    Ok(cargo_projects)
}

type ManifestEntry = (PathBuf, Option<WorkspaceManifest>);

/// Reads `dir`, records it if it holds a `Cargo.toml`, and recurses into its
/// subdirectories in parallel.
fn walk_dir(root: &Path, dir: &Path, filter: &IgnoreFilter) -> Vec<ManifestEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            report_walk_error(dir, &err);
            return Vec::new();
        }
    };

    let mut found = Vec::new();
    let mut sub_dirs = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                report_walk_error(dir, &err);
                continue;
            }
        };
        // `DirEntry::file_type` does not follow symbolic links.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let path = entry.path();
            if !filter.prunes_dir(relative_to(root, &path)) {
                sub_dirs.push(path);
            }
        } else if file_type.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| path_names_equal(name, "Cargo.toml"))
        {
            let path = entry.path();
            if !filter.skips_file(relative_to(root, &path)) {
                let manifest = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|contents| parse_workspace(&contents));
                found.push((dir.to_path_buf(), manifest));
            }
        }
    }

    found.par_extend(
        sub_dirs
            .par_iter()
            .flat_map_iter(|sub_dir| walk_dir(root, sub_dir, filter)),
    );
    found
}

fn relative_to<'p>(root: &Path, path: &'p Path) -> &'p Path {
    path.strip_prefix(root).unwrap_or(path)
}

fn report_walk_error(dir: &Path, err: &io::Error) {
    eprintln!(
        "Error: {}",
        format!("failed to read {}: {err}", dir.display()).red()
    );
}

/// Marks every project listed as a member of a workspace with [`CargoWorkspace::Parent`].
pub fn find_cargo_workspaces(cargo_projects: &HashMap<PathBuf, CargoProject>) {
    for (project_path, project) in cargo_projects {
        if let CargoWorkspace::WorkspaceMembers(sub_paths) = &*project.workspace.borrow() {
            for sub_path in sub_paths {
                // The project is already borrowed above, so a self-reference would panic.
                if sub_path == project_path {
                    continue;
                }
                if let Some(project) = cargo_projects.get(sub_path) {
                    // RefCell is used here to allow for interior mutability of the workspace
                    // As we need to have two borrows of cargo_projects, one being mutable
                    *project.workspace.borrow_mut() = CargoWorkspace::Parent(project_path.clone());
                }
            }
        }
    }
}

/// Outcome of cleaning a single project.
#[derive(Debug)]
pub enum CleanOutcome {
    Cleaned { stderr: String },
    SkippedWorkspaceMember { parent: PathBuf },
}

pub async fn run_cargo_clean(
    release_only: bool,
    doc_only: bool,
    project_path: PathBuf,
    project_info: CargoProject,
) -> Result<CleanOutcome, io::Error> {
    if let CargoWorkspace::Parent(parent) = &*project_info.workspace.borrow() {
        return Ok(CleanOutcome::SkippedWorkspaceMember {
            parent: parent.clone(),
        });
    }

    let output = tokio::process::Command::new("cargo")
        .current_dir(&project_path)
        .args(cargo_clean_args(release_only, doc_only))
        .output()
        .await?;
    Ok(CleanOutcome::Cleaned {
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(path: &str) -> PathBuf {
        PathBuf::from(path)
    }

    fn get_base_hashmap() -> HashMap<PathBuf, CargoProject> {
        HashMap::from([
            (
                p("/home/user/projects/project1"),
                CargoProject::new(CargoWorkspace::None),
            ),
            (
                p("/home/user/projects/project2"),
                CargoProject::new(CargoWorkspace::None),
            ),
            (
                p("/home/user/projects/project3"),
                CargoProject::new(CargoWorkspace::None),
            ),
            (
                p("/home/user/projects/project4"),
                CargoProject::new(CargoWorkspace::WorkspaceMembers(vec![
                    p("/home/user/projects/project1"),
                    p("/home/user/projects/project2"),
                ])),
            ),
        ])
    }

    fn workspace_of(map: &HashMap<PathBuf, CargoProject>, path: &str) -> CargoWorkspace {
        map.get(&p(path)).unwrap().workspace.borrow().clone()
    }

    #[test]
    fn find_cargo_workspaces_test() {
        let base_map = get_base_hashmap();

        find_cargo_workspaces(&base_map);

        assert_eq!(
            workspace_of(&base_map, "/home/user/projects/project1"),
            CargoWorkspace::Parent(p("/home/user/projects/project4"))
        );
        assert_eq!(
            workspace_of(&base_map, "/home/user/projects/project2"),
            CargoWorkspace::Parent(p("/home/user/projects/project4"))
        );
        assert_eq!(
            workspace_of(&base_map, "/home/user/projects/project3"),
            CargoWorkspace::None
        );
        assert_eq!(
            workspace_of(&base_map, "/home/user/projects/project4"),
            CargoWorkspace::WorkspaceMembers(vec![
                p("/home/user/projects/project1"),
                p("/home/user/projects/project2"),
            ])
        );
    }

    #[test]
    fn find_cargo_workspaces_ignores_missing_members() {
        let map = HashMap::from([(
            p("/ws"),
            CargoProject::new(CargoWorkspace::WorkspaceMembers(vec![p("/ws/missing")])),
        )]);

        find_cargo_workspaces(&map);

        assert_eq!(map.len(), 1);
        assert_eq!(
            workspace_of(&map, "/ws"),
            CargoWorkspace::WorkspaceMembers(vec![p("/ws/missing")])
        );
    }

    #[test]
    fn find_cargo_workspaces_skips_self_member() {
        // A workspace listing its own directory must not be borrowed mutably while borrowed.
        let map = HashMap::from([
            (
                p("/ws"),
                CargoProject::new(CargoWorkspace::WorkspaceMembers(vec![
                    p("/ws/."),
                    p("/ws/member"),
                ])),
            ),
            (p("/ws/member"), CargoProject::new(CargoWorkspace::None)),
        ]);

        find_cargo_workspaces(&map);

        assert_eq!(
            workspace_of(&map, "/ws/member"),
            CargoWorkspace::Parent(p("/ws"))
        );
        assert!(matches!(
            workspace_of(&map, "/ws"),
            CargoWorkspace::WorkspaceMembers(_)
        ));
    }

    fn manifest(members: &[&str], exclude: &[&str]) -> WorkspaceManifest {
        WorkspaceManifest {
            members: members.iter().map(|m| m.to_string()).collect(),
            exclude: exclude.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn parse_workspace_plain_package() {
        let toml = "[package]\nname = \"a\"\nversion = \"0.1.0\"\n";
        assert_eq!(parse_workspace(toml), None);
    }

    #[test]
    fn parse_workspace_with_members_and_exclude() {
        let toml = "[workspace]\nmembers = [\"crates/*\", \"b\"]\nexclude = [\"crates/old\"]\n";
        assert_eq!(
            parse_workspace(toml),
            Some(manifest(&["crates/*", "b"], &["crates/old"]))
        );
    }

    #[test]
    fn parse_workspace_without_members_does_not_panic() {
        // An empty `[workspace]` table is commonly used to opt out of a parent workspace.
        let toml = "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\n[workspace]\n";
        assert_eq!(parse_workspace(toml), None);
    }

    #[test]
    fn parse_workspace_invalid_toml_does_not_panic() {
        assert_eq!(parse_workspace("[workspace]\nmembers = ["), None);
    }

    #[test]
    fn parse_workspace_skips_non_string_members() {
        let toml = "[workspace]\nmembers = [\"a\", 1, true]\n";
        assert_eq!(parse_workspace(toml), Some(manifest(&["a"], &[])));
    }

    #[test]
    fn resolve_plain_members() {
        let resolved =
            resolve_workspace_members(Path::new("/r"), &manifest(&["b", "./a/"], &[]), &[]);
        assert_eq!(resolved, vec![p("/r/a"), p("/r/b")]);
    }

    #[test]
    fn resolve_members_drops_workspace_root() {
        let resolved =
            resolve_workspace_members(Path::new("/r"), &manifest(&[".", "./", "a"], &[]), &[]);
        assert_eq!(resolved, vec![p("/r/a")]);
    }

    #[test]
    fn resolve_glob_members() {
        let projects = [
            p("/r"),
            p("/r/crates/a"),
            p("/r/crates/b"),
            p("/r/crates/a/nested"),
            p("/r/tools/x"),
            p("/elsewhere/crates/c"),
        ];
        let resolved =
            resolve_workspace_members(Path::new("/r"), &manifest(&["crates/*"], &[]), &projects);
        assert_eq!(resolved, vec![p("/r/crates/a"), p("/r/crates/b")]);
    }

    #[test]
    fn resolve_recursive_glob_members() {
        let projects = [p("/r/crates/a"), p("/r/crates/a/nested"), p("/r/other")];
        let resolved =
            resolve_workspace_members(Path::new("/r"), &manifest(&["crates/**"], &[]), &projects);
        assert_eq!(resolved, vec![p("/r/crates/a"), p("/r/crates/a/nested")]);
    }

    #[test]
    fn resolve_members_honours_exclude() {
        let projects = [p("/r/crates/a"), p("/r/crates/old"), p("/r/crates/old/sub")];
        let resolved = resolve_workspace_members(
            Path::new("/r"),
            &manifest(&["crates/**", "crates/old"], &["crates/old"]),
            &projects,
        );
        assert_eq!(resolved, vec![p("/r/crates/a")]);
    }

    #[test]
    fn resolve_members_skips_invalid_glob() {
        let projects = [p("/r/a")];
        let resolved =
            resolve_workspace_members(Path::new("/r"), &manifest(&["[unclosed*"], &[]), &projects);
        assert!(resolved.is_empty());
    }

    #[test]
    fn literal_dir_pattern_detection() {
        assert_eq!(literal_dir_pattern("**/target/**"), Some("target"));
        assert_eq!(literal_dir_pattern("**/.venv/**"), Some(".venv"));
        assert_eq!(
            literal_dir_pattern("**/node_modules/**"),
            Some("node_modules")
        );
        assert_eq!(literal_dir_pattern("**/tar*/**"), None);
        assert_eq!(literal_dir_pattern("**/a/b/**"), None);
        assert_eq!(literal_dir_pattern("target/**"), None);
        assert_eq!(literal_dir_pattern("**/(?i)target/**"), None);
        assert_eq!(literal_dir_pattern("**/../**"), None);
    }

    #[test]
    fn literal_dir_fast_path_agrees_with_wax() {
        let candidates = [
            "target",
            "a/target",
            "a/b/target",
            "targets",
            "a/target2",
            "a/my-target",
            "Target",
            "a/target/Cargo.toml",
            "a/.venv",
            "a/venv",
        ];
        for pattern in ["**/target/**", "**/.venv/**"] {
            let fast = IgnoreFilter::new(&[pattern]).unwrap();
            assert_eq!(fast.dir_names, vec![literal_dir_pattern(pattern).unwrap()]);
            let glob = wax::Glob::new(pattern).unwrap();
            assert!(glob.is_exhaustive().is_always());
            for candidate in candidates {
                // Walk order: every ancestor is checked before its children.
                let path = Path::new(candidate);
                let fast_pruned = path
                    .ancestors()
                    .filter(|a| !a.as_os_str().is_empty())
                    .any(|a| fast.prunes_dir(a));
                assert_eq!(
                    fast_pruned,
                    glob.is_match(path),
                    "pattern {pattern}, path {candidate}"
                );
            }
        }
    }

    #[test]
    fn nonexhaustive_pattern_skips_only_matching_manifest() {
        let filter = IgnoreFilter::new(&["**/skip/Cargo.toml"]).unwrap();
        assert!(filter.dir_names.is_empty());
        assert!(!filter.prunes_dir(Path::new("a/skip")));
        assert!(filter.skips_file(Path::new("a/skip/Cargo.toml")));
        assert!(!filter.skips_file(Path::new("a/keep/Cargo.toml")));
    }

    #[test]
    fn resolve_ignored_patterns_defaults() {
        assert_eq!(resolve_ignored_patterns(None), DEFAULT_IGNORED_PATTERNS);
    }

    #[test]
    fn resolve_ignored_patterns_trims_and_drops_blank() {
        let user = vec![" **/venv/** ".to_owned(), "".to_owned(), "   ".to_owned()];
        assert_eq!(resolve_ignored_patterns(Some(&user)), vec!["**/venv/**"]);
    }

    #[test]
    fn resolve_ignored_patterns_empty_string_disables_ignoring() {
        let user = vec![String::new()];
        assert!(resolve_ignored_patterns(Some(&user)).is_empty());
    }

    #[test]
    fn cargo_clean_args_variants() {
        assert_eq!(cargo_clean_args(false, false), vec!["clean"]);
        assert_eq!(cargo_clean_args(true, false), vec!["clean", "--release"]);
        assert_eq!(cargo_clean_args(false, true), vec!["clean", "--doc"]);
        // release takes precedence (clap forbids both, but the function must stay sane)
        assert_eq!(cargo_clean_args(true, true), vec!["clean", "--release"]);
    }

    #[tokio::test]
    async fn run_cargo_clean_skips_workspace_members() {
        let project = CargoProject::new(CargoWorkspace::Parent(p("/ws")));
        let outcome = run_cargo_clean(false, false, p("/ws/member"), project)
            .await
            .unwrap();
        assert!(
            matches!(outcome, CleanOutcome::SkippedWorkspaceMember { parent } if parent == p("/ws"))
        );
    }
}
