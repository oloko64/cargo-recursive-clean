use clap::Parser;

#[derive(Parser)]
#[command(version)]
/// A CLI app that cleans all Rust projects recursively given a base directory
pub struct Arguments {
    /// The directory to search for cargo projects, defaults to current directory
    #[arg(default_value = ".")]
    pub base_dir: String,

    /// Clean only the release build artifacts
    #[arg(short, long, group = "clean_type")]
    pub release: bool,

    /// Clean only the documentation build artifacts
    #[arg(short, long, group = "clean_type")]
    pub doc: bool,
}
