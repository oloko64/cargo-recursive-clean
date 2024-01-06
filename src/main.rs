mod arg_parser;

use itertools::Itertools;
use owo_colors::OwoColorize;
use std::{
    cell::RefCell,
    collections::HashMap,
    io::{self, Write},
    path::PathBuf,
};
use tokio::task::JoinSet;

use crate::arg_parser::get_args;

const DEFAULT_IGNORED_PATTERNS: &[&str] = &["**/node_modules/**", "**/target/**"];
const ASK_CONFIRMATION_LIMIT: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CargoProject {
    workspace: RefCell<CargoWorkspace>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum CargoWorkspace {
    WorkspaceMembers(Vec<PathBuf>),
    Parent(PathBuf),
    None,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run()?;

    Ok(())
}

#[tokio::main]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if get_args().release {
        println!("{}", "Cleaning only release artifacts...".magenta().bold());
    } else if get_args().doc {
        println!(
            "{}",
            "Cleaning only documentation artifacts...".magenta().bold()
        );
    } else {
        println!("{}", "Cleaning all artifacts...".magenta().bold());
    }
    let cargo_projects = all_cargo_projects()?;

    println!(
        "Found {} cargo projects under: {}\n",
        cargo_projects.len().green(),
        get_args().path.green()
    );

    if cargo_projects.len() > ASK_CONFIRMATION_LIMIT && !get_args().yes && !get_args().dry_run {
        if ask_confirmation(&format!(
            "Are you sure you want to clean all {} projects? (y/N)",
            cargo_projects.len().red()
        )) {
            println!(
                "Cleaning all {} projects...\n",
                cargo_projects.len().green()
            );
        } else {
            println!("Exiting...");
            return Ok(());
        }
    }

    if cargo_projects.is_empty() {
        println!("{}", "No projects found, exiting...".yellow());
        return Ok(());
    }

    if get_args().dry_run {
        println!(
            "{}\n{}\n\n{} project(s) would be cleaned",
            "Dry run, nothing will be cleaned.\n\nThe following projects would be cleaned:".green(),
            cargo_projects
                .keys()
                .map(|path| path.display().to_string())
                .join("\n"),
            cargo_projects.len().green()
        );
    } else {
        clean_projects(cargo_projects).await;
    }

    Ok(())
}

fn ask_confirmation(msg: &str) -> bool {
    let mut input = String::new();
    print!("{msg} ");
    io::stdout().flush().unwrap();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_lowercase() == "y"
}

async fn clean_projects(cargo_projects: HashMap<PathBuf, CargoProject>) {
    let mut handles = JoinSet::new();
    let release_only = get_args().release;
    let doc_only = get_args().doc;
    for project in cargo_projects {
        handles.spawn(run_cargo_clean(release_only, doc_only, project));
    }

    let mut cleaned_successful_count = 0;
    while let Some(handle) = handles.join_next().await {
        if let Err(err) = handle.unwrap() {
            println!("Error: {}", err.red());
        }
        cleaned_successful_count += 1;
    }

    println!("\nCleaned {} projects", cleaned_successful_count.green());
}

async fn run_cargo_clean(
    release_only: bool,
    doc_only: bool,
    (project_path, project_info): (PathBuf, CargoProject),
) -> Result<(), io::Error> {
    let mut args = vec!["clean"];

    if let CargoWorkspace::Parent(parent) = &*project_info.workspace.borrow() {
        println!(
            "Skipping: {} ->> {} {}",
            project_path.as_path().display().cyan(),
            parent.display().cyan(),
            "(workspace)".yellow()
        );
        return Ok(());
    }

    if release_only {
        args.push("--release");
    } else if doc_only {
        args.push("--doc");
    }
    let output = tokio::process::Command::new("cargo")
        .current_dir(&project_path)
        .args(&args)
        .output()
        .await?;
    println!(
        "Cleaned: {} ->> {}",
        project_path.as_path().display().cyan(),
        String::from_utf8_lossy(&output.stderr).trim().magenta()
    );
    Ok(())
}

fn all_cargo_projects() -> Result<HashMap<PathBuf, CargoProject>, Box<dyn std::error::Error>> {
    let mut patterns = vec![];
    if let Some(ignored_patterns) = &get_args().ignored_patterns {
        patterns.extend(ignored_patterns.iter().filter_map(|pattern| {
            if pattern.trim().is_empty() {
                None
            } else {
                Some(pattern.as_str().trim())
            }
        }));
    } else {
        patterns.extend(DEFAULT_IGNORED_PATTERNS);
    }
    if !patterns.is_empty() {
        println!("Ignored patterns: {:?}", &patterns.green());
    }
    let glob = wax::Glob::new("**/Cargo.toml")?;
    let cargo_projects = glob
        .walk(&get_args().path)
        .not(patterns)?
        .filter_map(|entry| {
            // print error if we get an error
            let entry = entry
                .map_err(|err| {
                    eprintln!("Error: {}", err.to_string().red());
                    err
                })
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
        .sorted()
        .map(|path| {
            let mut workspace = CargoWorkspace::None;
            if let Ok(toml) = std::fs::read_to_string(path.join("Cargo.toml")) {
                if toml.contains("[workspace]") {
                    let parsed =
                        toml::from_str::<toml::Value>(&toml).expect("Failed to parse toml");
                    let sub_workspaces = parsed["workspace"]["members"]
                        .as_array()
                        .cloned()
                        .expect("Failed to get workspace members")
                        .into_iter()
                        .filter_map(|member| member.as_str().map(ToOwned::to_owned))
                        .map(|member| path.join(member))
                        .collect::<Vec<PathBuf>>();
                    workspace = CargoWorkspace::WorkspaceMembers(sub_workspaces);
                }
            }
            (
                path,
                CargoProject {
                    workspace: RefCell::new(workspace),
                },
            )
        })
        .collect::<HashMap<PathBuf, CargoProject>>();

    find_cargo_workspaces(&cargo_projects);

    Ok(cargo_projects)
}

fn find_cargo_workspaces(cargo_projects: &HashMap<PathBuf, CargoProject>) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn get_base_hashmap() -> HashMap<PathBuf, CargoProject> {
        let mut cargo_projects = HashMap::new();
        cargo_projects.insert(
            std::path::PathBuf::from("/home/user/projects/project1"),
            CargoProject {
                workspace: std::cell::RefCell::new(CargoWorkspace::None),
            },
        );
        cargo_projects.insert(
            std::path::PathBuf::from("/home/user/projects/project2"),
            CargoProject {
                workspace: std::cell::RefCell::new(CargoWorkspace::None),
            },
        );
        cargo_projects.insert(
            std::path::PathBuf::from("/home/user/projects/project3"),
            CargoProject {
                workspace: std::cell::RefCell::new(CargoWorkspace::None),
            },
        );
        cargo_projects.insert(
            std::path::PathBuf::from("/home/user/projects/project4"),
            CargoProject {
                workspace: std::cell::RefCell::new(CargoWorkspace::WorkspaceMembers(vec![
                    std::path::PathBuf::from("/home/user/projects/project1"),
                    std::path::PathBuf::from("/home/user/projects/project2"),
                ])),
            },
        );

        cargo_projects
    }

    #[test]
    fn find_cargo_workspaces_test() {
        let base_map = get_base_hashmap();

        find_cargo_workspaces(&base_map);

        assert_eq!(
            *base_map
                .get(&std::path::PathBuf::from("/home/user/projects/project1"))
                .unwrap()
                .workspace
                .borrow(),
            CargoWorkspace::Parent(std::path::PathBuf::from("/home/user/projects/project4"))
        );
        assert_eq!(
            *base_map
                .get(&std::path::PathBuf::from("/home/user/projects/project2"))
                .unwrap()
                .workspace
                .borrow(),
            CargoWorkspace::Parent(std::path::PathBuf::from("/home/user/projects/project4"))
        );
        assert_eq!(
            *base_map
                .get(&std::path::PathBuf::from("/home/user/projects/project3"))
                .unwrap()
                .workspace
                .borrow(),
            CargoWorkspace::None
        );
        assert_eq!(
            *base_map
                .get(&std::path::PathBuf::from("/home/user/projects/project4"))
                .unwrap()
                .workspace
                .borrow(),
            CargoWorkspace::WorkspaceMembers(vec![
                std::path::PathBuf::from("/home/user/projects/project1"),
                std::path::PathBuf::from("/home/user/projects/project2"),
            ])
        );
    }
}
