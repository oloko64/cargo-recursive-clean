use clap::Parser;

#[derive(Parser)]
#[command(version)]
/// A CLI app that cleans all Rust projects recursively given a base directory
pub struct Arguments {
    /// The directory to search for cargo projects, defaults to current directory
    #[arg(short, long, default_value = ".")]
    pub path: String,

    /// Clean only the release build artifacts
    #[arg(short, long, group = "clean_type")]
    pub release: bool,

    /// Clean only the documentation build artifacts
    #[arg(short, long, group = "clean_type")]
    pub doc: bool,

    /// Dry run, don't actually clean anything, just print what directories would be cleaned
    #[arg(long = "dry")]
    pub dry_run: bool,

    /// Pass confirmation limit without any prompt
    #[arg(short, long)]
    pub yes: bool,

    /// Ignored patterns
    #[arg(long, value_delimiter = ',')]
    pub ignored_patterns: Option<Vec<String>>,
}
