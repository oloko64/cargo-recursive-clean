# Rust Recursive Project Cleaner

A CLI app that cleans all Rust projects recursively given a base directory.

## Build

```bash
cargo build --release
```
This will build the app in release mode, and the binary will be available in the `target/release/` directory. Or you can install it:

```bash
cargo install --path .
```

## Usage

To clean all Rust projects recursively in the current directory:

```bash
cargo-recursive-clean
```

To clean all Rust projects recursively in a specific directory:

```bash
cargo-recursive-clean </path/to/directory>
```

You can also specify to only clean release or doc artifacts:

```bash
cargo-recursive-clean --release
cargo-recursive-clean --doc
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.