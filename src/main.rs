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
            cargo_projects.iter().map(|p| p.display()).join("\n"),
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

async fn clean_projects(cargo_projects: Vec<PathBuf>) {
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
    project: PathBuf,
) -> Result<(), io::Error> {
    let mut args = vec!["clean"];
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
        project.as_path().display().green(),
        String::from_utf8_lossy(&output.stderr).trim().yellow()
    );
    Ok(())
}

fn all_cargo_projects() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
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
        println!("Ignored patterns: {:?}", &patterns);
    }
    let glob = wax::Glob::new("**/Cargo.toml")?;
    let cargo_projects = glob
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
        .collect::<Vec<PathBuf>>();

    Ok(cargo_projects)
}
