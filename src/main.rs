mod arg_parser;

use clap::Parser;
use itertools::Itertools;
use owo_colors::OwoColorize;
use std::{io::{self, Write}, path::PathBuf};
use tokio::task::JoinSet;

const DEFAULT_IGNORED_PATTERNS: &[&str] = &["!**/node_modules/**"];
const ASK_CONFIRMATION_LIMIT: usize = 500;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = arg_parser::Arguments::parse();

    run(&args)?;

    Ok(())
}

#[tokio::main]
async fn run(args: &arg_parser::Arguments) -> Result<(), Box<dyn std::error::Error>> {
    if args.release {
        println!("{}", "Cleaning only release artifacts...".magenta());
    } else if args.doc {
        println!("{}", "Cleaning only documentation artifacts...".magenta());
    } else {
        println!("{}", "Cleaning all artifacts...".magenta());
    }
    let cargo_projects = all_cargo_projects(&args.base_dir, &args.ignored_patterns)?;

    println!(
        "Found {} cargo projects under: {}\n",
        cargo_projects.len().green(),
        args.base_dir.green()
    );

    if cargo_projects.len() > ASK_CONFIRMATION_LIMIT && !args.yes && !args.dry_run {
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
            std::process::exit(0);
        }
    }

    if cargo_projects.is_empty() {
        println!("{}", "No projects found, exiting...".yellow());
        std::process::exit(0);
    }

    if args.dry_run {
        println!("{}", "Dry run, nothing will be cleaned.\n".magenta());
        println!("{}", "The following projects would be cleaned:".green());
        println!(
            "{}\n",
            cargo_projects.iter().map(|p| p.display()).join("\n")
        );
        println!(
            "{} project(s) would be cleaned",
            cargo_projects.len().green()
        );
    } else {
        clean_projects(args, cargo_projects).await;
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

async fn clean_projects(args: &arg_parser::Arguments, cargo_projects: Vec<PathBuf>) {
    let mut handles = JoinSet::new();
    let release_only = args.release;
    let doc_only = args.doc;
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
        "Cleaned: {} {}",
        project.as_path().display().green(),
        String::from_utf8_lossy(&output.stdout).yellow()
    );
    Ok(())
}

fn all_cargo_projects(
    base_dir: &str,
    ignored_patterns: &Option<Vec<String>>,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut patterns = vec!["**/Cargo.toml"];
    if let Some(ignored_patterns) = ignored_patterns {
        patterns.extend(ignored_patterns.iter().filter_map(|pattern| {
            if !pattern.trim().is_empty() && pattern.trim().starts_with('!') {
                Some(pattern.as_str().trim())
            } else if !pattern.trim().starts_with('!') {
                eprintln!(
                    "Error on pattern: {} | Reason: Patterns must start with {}.",
                    pattern.red(),
                    "'!'".yellow()
                );
                std::process::exit(1);
            } else {
                None
            }
        }));
    } else {
        patterns.extend(DEFAULT_IGNORED_PATTERNS);
    }
    if patterns.len() > 1 {
        println!("Ignored patterns: {:?}", &patterns[1..]);
    }
    let cargo_projects = globwalk::GlobWalkerBuilder::from_patterns(base_dir, &patterns)
        .build()?
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
