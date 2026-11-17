use std::{env::args, sync::OnceLock};

use clap::Parser;

#[derive(Parser)]
#[command(version)]
/// A CLI app that cleans all Rust projects recursively given a base directory
pub struct Arguments {
    /// The directory to search for cargo projects, defaults to current directory
    #[arg(default_value = ".")]
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

pub fn get_args() -> &'static Arguments {
    static INSTANCE: OnceLock<Arguments> = OnceLock::new();

    INSTANCE.get_or_init(|| {
        // We need to skip the first argument when using the cargo extend feature otherwise it will fail to parse the arguments
        let mut raw_args = args();
        // This is a hacky way to make the app work under cargo extend, the name must match the name of the binary in Cargo.toml without the `cargo-` prefix
        if let Some("recursive-clean") = std::env::args().nth(1).as_deref() {
            raw_args.next();
        }
        // Now we can parse the arguments without having to worry about the first argument
        Arguments::parse_from(raw_args)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Arguments, clap::Error> {
        Arguments::try_parse_from(
            std::iter::once("cargo-recursive-clean").chain(args.iter().copied()),
        )
    }

    #[test]
    fn defaults() {
        let args = parse(&[]).unwrap();
        assert_eq!(args.path, ".");
        assert!(!args.release && !args.doc && !args.dry_run && !args.yes);
        assert!(args.ignored_patterns.is_none());
    }

    #[test]
    fn all_flags() {
        let args = parse(&["/tmp/x", "-r", "--dry", "-y"]).unwrap();
        assert_eq!(args.path, "/tmp/x");
        assert!(args.release && args.dry_run && args.yes);
        assert!(!args.doc);
    }

    #[test]
    fn release_and_doc_are_mutually_exclusive() {
        assert!(parse(&["--release", "--doc"]).is_err());
    }

    #[test]
    fn ignored_patterns_are_comma_separated() {
        let args = parse(&["--ignored-patterns", "**/a/**,**/b/**"]).unwrap();
        assert_eq!(
            args.ignored_patterns.unwrap(),
            vec!["**/a/**".to_owned(), "**/b/**".to_owned()]
        );
    }

    #[test]
    fn empty_ignored_patterns() {
        let args = parse(&["--ignored-patterns", ""]).unwrap();
        assert_eq!(args.ignored_patterns.unwrap(), vec![String::new()]);
    }
}
