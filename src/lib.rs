pub mod arg_parser;

use owo_colors::OwoColorize;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    cell::RefCell,
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};
use wax::{
    Program,
    walk::{Entry, FileIterator},
};

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
/// Paths under any `exclude` entry are dropped.
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

    resolved.retain(|member| !excluded.iter().any(|exclude| member.starts_with(exclude)));
    resolved.sort();
    resolved.dedup();
    resolved
}

/// Walks `root` and returns every directory containing a `Cargo.toml`, skipping
/// paths matching `ignored_patterns`. Workspace members are marked with their parent.
///
/// Entries that cannot be read (e.g. permission denied) are reported on stderr and skipped.
pub fn find_cargo_projects(
    root: impl AsRef<Path>,
    ignored_patterns: &[&str],
) -> Result<HashMap<PathBuf, CargoProject>, Box<dyn std::error::Error>> {
    let glob = wax::Glob::new("**/Cargo.toml")?;
    let walk = glob.walk(root.as_ref());

    let mut manifest_dirs: Vec<PathBuf> = if ignored_patterns.is_empty() {
        collect_manifest_dirs(walk)
    } else {
        collect_manifest_dirs(walk.not(wax::any(ignored_patterns.iter().copied()))?)
    };
    manifest_dirs.sort();

    let manifests: Vec<(PathBuf, Option<WorkspaceManifest>)> = manifest_dirs
        .par_iter()
        .map(|path| {
            let manifest = std::fs::read_to_string(path.join("Cargo.toml"))
                .ok()
                .and_then(|contents| parse_workspace(&contents));
            (path.clone(), manifest)
        })
        .collect();

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

fn collect_manifest_dirs<I, E>(entries: I) -> Vec<PathBuf>
where
    I: Iterator<Item = Result<E, wax::walk::WalkError>>,
    E: Entry,
{
    entries
        .filter_map(|entry| {
            let entry = entry
                .inspect_err(|err| eprintln!("Error: {}", err.to_string().red()))
                .ok()?;
            if entry.file_type().is_file() {
                Some(
                    entry
                        .path()
                        .parent()
                        .expect("Failed to find parent directory")
                        .to_path_buf(),
                )
            } else {
                None
            }
        })
        .collect()
}

/// Marks every project listed as a member of a workspace with [`CargoWorkspace::Parent`].
pub fn find_cargo_workspaces(cargo_projects: &HashMap<PathBuf, CargoProject>) {
    for (project_path, project) in cargo_projects {
        if let CargoWorkspace::WorkspaceMembers(sub_paths) = &*project.workspace.borrow() {
            for sub_path in sub_paths {
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
