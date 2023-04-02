mod arg_parser;

use clap::Parser;
use itertools::Itertools;
use owo_colors::OwoColorize;
use std::{io, path::PathBuf};
use tokio::task::JoinSet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = arg_parser::Arguments::parse();

    run(&args)?;

    Ok(())
}

#[tokio::main]
async fn run(args: &arg_parser::Arguments) -> Result<(), Box<dyn std::error::Error>> {
    let cargo_projects = all_cargo_folders(&args.base_dir)?;
    if args.release {
        println!("{}", "Cleaning only release artifacts".magenta());
    } else if args.doc {
        println!("{}", "Cleaning only documentation artifacts".magenta());
    } else {
        println!("{}", "Cleaning all artifacts".magenta());
    }

    println!(
        "Found {} cargo projects under: {}\n",
        cargo_projects.len(),
        args.base_dir.green()
    );

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

    Ok(())
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

fn all_cargo_folders(base_dir: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let cargo_projects = globwalk::GlobWalkerBuilder::from_patterns(base_dir, &["**/Cargo.toml"])
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
