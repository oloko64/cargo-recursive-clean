# Rust Recursive Project Cleaner

A CLI app that cleans all Rust projects recursively given a base directory.

## Install

```bash
cargo install cargo-recursive-clean
```

## Usage

To clean all Rust projects recursively in the current directory:

```bash
cargo-recursive-clean
```

You can also use the [extending cargo](https://doc.rust-lang.org/book/ch14-05-extending-cargo.html) feature:

```bash
cargo recursive-clean
```

All the option below are also available with this feature.

To clean all Rust projects recursively in a specific directory:

```bash
cargo-recursive-clean <path-to-directory>
```

You can also specify to only clean release or doc artifacts:

```bash
cargo-recursive-clean --release
cargo-recursive-clean --doc
```

### Dry run

You can also specify to only print the directories that would be cleaned, without actually cleaning them:

```bash
cargo-recursive-clean --dry
```

### Ignoring patterns

You can specify a list of patterns to ignore when cleaning. This is useful if you have a project that you don't want to clean, or if you have a project that you want to clean but it's not a Rust project. By default, the following patterns are ignored: `'**/node_modules/**','**/target/**'`.

You can also specify a list of patterns to ignore when cleaning, for example:

```bash
cargo-recursive-clean --ignored-patterns '**/node_modules/**,**/venv/**'
```

This will ignore all `node_modules` and `venv` directories.

To not ignore any patterns, you can simply pass an empty string:

```bash
cargo-recursive-clean --ignored-patterns ''
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.