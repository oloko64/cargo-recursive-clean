use std::{collections::HashMap, fs, hint::black_box, path::PathBuf};

use cargo_recursive_clean::{
    CargoProject, CargoWorkspace, DEFAULT_IGNORED_PATTERNS, find_cargo_projects,
    find_cargo_workspaces, parse_workspace,
};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use tempfile::TempDir;

/// Creates `count` standalone projects plus one workspace with `count / 4` members,
/// and a `target/` dir in each standalone project holding a stray `Cargo.toml`
/// that the default ignore patterns must skip.
fn build_tree(count: usize) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for i in 0..count {
        let project = root.join(format!("group{}/project{i}", i % 8));
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir_all(project.join("target/debug/build/dep")).unwrap();
        fs::write(
            project.join("Cargo.toml"),
            format!("[package]\nname = \"p{i}\"\nversion = \"0.1.0\"\n"),
        )
        .unwrap();
        fs::write(project.join("target/debug/build/dep/Cargo.toml"), "").unwrap();
    }
    let ws = root.join("workspace");
    let members: Vec<String> = (0..count / 4).map(|i| format!("\"m{i}\"")).collect();
    fs::create_dir_all(&ws).unwrap();
    fs::write(
        ws.join("Cargo.toml"),
        format!("[workspace]\nmembers = [{}]\n", members.join(", ")),
    )
    .unwrap();
    for i in 0..count / 4 {
        let member = ws.join(format!("m{i}"));
        fs::create_dir_all(&member).unwrap();
        fs::write(
            member.join("Cargo.toml"),
            format!("[package]\nname = \"m{i}\"\nversion = \"0.1.0\"\n"),
        )
        .unwrap();
    }
    dir
}

fn bench_find_cargo_projects(c: &mut Criterion) {
    let mut group = c.benchmark_group("find_cargo_projects");
    group.sample_size(20);
    for count in [10, 100, 500] {
        let tree = build_tree(count);
        group.bench_with_input(
            BenchmarkId::new("default_ignores", count),
            &tree,
            |b, tree| {
                b.iter(|| find_cargo_projects(tree.path(), DEFAULT_IGNORED_PATTERNS).unwrap())
            },
        );
        group.bench_with_input(BenchmarkId::new("no_ignores", count), &tree, |b, tree| {
            b.iter(|| find_cargo_projects(tree.path(), &[]).unwrap())
        });
    }
    group.finish();
}

fn bench_find_cargo_workspaces(c: &mut Criterion) {
    let mut group = c.benchmark_group("find_cargo_workspaces");
    for count in [100, 1_000, 10_000] {
        let members: Vec<PathBuf> = (0..count / 2)
            .map(|i| PathBuf::from(format!("/ws/m{i}")))
            .collect();
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            b.iter_batched(
                || {
                    let mut map: HashMap<PathBuf, CargoProject> = (0..count)
                        .map(|i| {
                            let path = if i < count / 2 {
                                format!("/ws/m{i}")
                            } else {
                                format!("/other/p{i}")
                            };
                            (PathBuf::from(path), CargoProject::new(CargoWorkspace::None))
                        })
                        .collect();
                    map.insert(
                        PathBuf::from("/ws"),
                        CargoProject::new(CargoWorkspace::WorkspaceMembers(members.clone())),
                    );
                    map
                },
                |map| {
                    find_cargo_workspaces(&map);
                    map
                },
                criterion::BatchSize::LargeInput,
            )
        });
    }
    group.finish();
}

fn bench_parse_workspace(c: &mut Criterion) {
    let package = "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n";
    let members: Vec<String> = (0..200).map(|i| format!("\"crates/c{i}\"")).collect();
    let workspace = format!("[workspace]\nmembers = [{}]\n", members.join(", "));

    let mut group = c.benchmark_group("parse_workspace");
    group.bench_function("package", |b| {
        b.iter(|| parse_workspace(black_box(package)))
    });
    group.bench_function("workspace_200_members", |b| {
        b.iter(|| parse_workspace(black_box(&workspace)))
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_find_cargo_projects,
    bench_find_cargo_workspaces,
    bench_parse_workspace
);
criterion_main!(benches);
