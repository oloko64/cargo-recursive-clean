mod arg_parser;

use itertools::Itertools;
use owo_colors::OwoColorize;
use std::{
    io::{self, Write},
    path::PathBuf,
};
use tokio::task::JoinSet;

use crate::arg_parser::get_args;

const DEFAULT_IGNORED_PATTERNS: &[&str] = &["**/node_modules/**", "**/target/**"];
const ASK_CONFIRMATION_LIMIT: usize = 500;

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
            cargo_projects.iter().map(ToString::to_string).join("\n"),
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

async fn clean_projects(cargo_projects: Vec<CargoProject>) {
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
    project: CargoProject,
) -> Result<(), io::Error> {
    let mut args = vec!["clean"];
    let CargoProject {
        path: project,
        workspace: is_workspace,
    } = project;

    if let CargoWorkspace::Parent(parent) = is_workspace {
        println!(
            "Skipping: {} ->> {} {}",
            project.as_path().display().cyan(),
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
        .current_dir(&project)
        .args(&args)
        .output()
        .await?;
    println!(
        "Cleaned: {} ->> {}",
        project.as_path().display().cyan(),
        String::from_utf8_lossy(&output.stderr).trim().magenta()
    );
    Ok(())
}

fn all_cargo_projects() -> Result<Vec<CargoProject>, Box<dyn std::error::Error>> {
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
    let mut cargo_projects = glob
        .walk(&get_args().path)
        .not(patterns)?
        .filter_map(|entry| {
            let entry = entry.expect("Failed to read entry");
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
                    let members = parsed["workspace"]["members"]
                        .as_array()
                        .cloned()
                        .expect("Failed to parse members")
                        .into_iter()
                        .filter_map(|member| member.as_str().map(ToOwned::to_owned))
                        .map(|member| path.join(member))
                        .collect::<Vec<PathBuf>>();
                    workspace = CargoWorkspace::WorkspaceMembers(members);
                }
            }
            CargoProject { path, workspace }
        })
        .collect::<Vec<CargoProject>>();

    let mut sub_workspace_cargo_projects = vec![];
    for project in &cargo_projects {
        if let CargoWorkspace::WorkspaceMembers(members) = &project.workspace {
            for member in members {
                if let Some(cargo_project) = cargo_projects
                    .iter()
                    .find(|project| &project.path == member)
                {
                    sub_workspace_cargo_projects.push(CargoProject {
                        path: cargo_project.path.clone(),
                        workspace: CargoWorkspace::Parent(project.path.clone()),
                    });
                }
            }
        }
    }

    for project in &mut cargo_projects {
        for sub_workspace_cargo_project in &sub_workspace_cargo_projects {
            if &project.path == &sub_workspace_cargo_project.path {
                project.workspace = sub_workspace_cargo_project.workspace.clone();
            }
        }
    }

    Ok(cargo_projects)
}

#[derive(Debug, Clone)]
struct CargoProject {
    path: PathBuf,
    workspace: CargoWorkspace,
}

#[derive(Debug, Clone)]
enum CargoWorkspace {
    WorkspaceMembers(Vec<PathBuf>),
    Parent(PathBuf),
    None,
}

impl std::fmt::Display for CargoProject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.workspace {
            CargoWorkspace::WorkspaceMembers(_) => {
                write!(f, "{} {}", self.path.display(), "(workspace)".yellow())
            }
            CargoWorkspace::Parent(_) => {
                write!(f, "{} {}", self.path.display(), "(parent)".yellow())
            }
            CargoWorkspace::None => write!(f, "{}", self.path.display()),
        }
    }
}
