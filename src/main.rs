use cargo_recursive_clean::{
    ASK_CONFIRMATION_LIMIT, CargoProject, CleanOutcome, arg_parser::get_args, find_cargo_projects,
    resolve_ignored_patterns, run_cargo_clean,
};
use itertools::Itertools;
use owo_colors::OwoColorize;
use std::{
    collections::HashMap,
    io::{self, Write},
    path::PathBuf,
};
use tokio::task::JoinSet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run()?;

    Ok(())
}

#[tokio::main(flavor = "multi_thread")]
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

    let patterns = resolve_ignored_patterns(get_args().ignored_patterns.as_deref());
    if !patterns.is_empty() {
        println!("Ignored patterns: {:?}", patterns.green());
    }
    let cargo_projects = find_cargo_projects(&get_args().path, &patterns)?;

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
    for (project_path, project) in cargo_projects {
        handles.spawn(async move {
            let outcome =
                run_cargo_clean(release_only, doc_only, project_path.clone(), project).await;
            report_outcome(&project_path, outcome)
        });
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

fn report_outcome(
    project_path: &std::path::Path,
    outcome: Result<CleanOutcome, io::Error>,
) -> Result<(), io::Error> {
    match outcome? {
        CleanOutcome::SkippedWorkspaceMember { parent } => println!(
            "Skipping: {} ->> {} {}",
            project_path.display().cyan(),
            parent.display().cyan(),
            "(workspace)".yellow()
        ),
        CleanOutcome::Cleaned { stderr } => println!(
            "Cleaned: {} ->> {}",
            project_path.display().cyan(),
            stderr.magenta()
        ),
    }
    Ok(())
}
