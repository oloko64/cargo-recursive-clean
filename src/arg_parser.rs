use clap::Parser;

#[derive(Parser)]
#[command(version)]
/// A utility to clean all cargo projects under a given directory recursively
pub struct Arguments {
    /// The directory to search for cargo projects
    pub base_dir: String,

    /// Clean only the release build artifacts
    #[arg(short, long, group = "clean_type")]
    pub release: bool,

    /// Clean only the documentation build artifacts
    #[arg(short, long, group = "clean_type")]
    pub doc: bool,
}
