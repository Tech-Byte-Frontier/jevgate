//! What the sources may not do, which the compiler does not check. Unused
//! code is deleted and long parameter lists are grouped: no source may
//! suppress those lints, with `allow` or `expect`; Cargo.toml denies them.
//! And Git starts only where a test's Git is kept out of the repository
//! running the tests.
use std::{
    fs,
    path::{Path, PathBuf},
};

const NEVER_SUPPRESSED: [&str; 4] = ["dead_code", "unused", "too_many_arguments", "complexity"];

/// The files that may start Git: the check's one constructor, which drops the
/// variables that point Git at a repository in unit tests, and the tests'
/// helper, which drops them for every other test's Git and `jevgate`.
const STARTS_GIT: [&str; 2] = ["src/revision.rs", "tests/support/git.rs"];

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// Every Rust source under `src/` and `tests/`, named by its path from the
/// crate root with `/` separators, with its text.
fn sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    rust_files(&root.join("tests"), &mut files);
    files
        .into_iter()
        .map(|path| {
            let name = path.strip_prefix(root).unwrap().to_string_lossy();
            (name.replace('\\', "/"), fs::read_to_string(&path).unwrap())
        })
        .collect()
}

/// Lint names inside `#[allow(…)]`, `#[expect(…)]` and their `#![…]` forms.
fn suppressed(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in source.lines().map(str::trim) {
        let Some(attribute) = line.strip_prefix("#![").or_else(|| line.strip_prefix("#[")) else {
            continue;
        };
        for level in ["allow(", "expect("] {
            if let Some(list) = attribute.strip_prefix(level) {
                let list = list.split(')').next().unwrap_or("");
                names.extend(
                    list.split(',')
                        .map(|name| name.trim().trim_start_matches("clippy::"))
                        .filter(|name| !name.contains('='))
                        .map(str::to_string),
                );
            }
        }
    }
    names
}

#[test]
fn no_source_suppresses_unused_code_or_long_parameter_lists() {
    let mut violations = Vec::new();
    for (path, source) in sources() {
        for name in suppressed(&source) {
            if NEVER_SUPPRESSED.contains(&name.as_str()) {
                violations.push(format!("{path}: {name}"));
            }
        }
    }
    assert!(violations.is_empty(), "{violations:#?}");
}

#[test]
fn suppressed_lints_are_read_from_both_attribute_forms() {
    let source =
        "#[allow(clippy::too_many_arguments)]\nfn f() {}\n#![expect(dead_code, reason = \"x\")]\n";
    assert_eq!(suppressed(source), ["too_many_arguments", "dead_code"]);
}

#[test]
fn git_starts_only_where_tests_keep_it_out_of_the_repository_running_them() {
    let elsewhere: Vec<String> = sources()
        .into_iter()
        .filter(|(path, source)| {
            source.contains("Command::new(\"git\")") && !STARTS_GIT.contains(&path.as_str())
        })
        .map(|(path, _)| path)
        .collect();
    assert!(
        elsewhere.is_empty(),
        "Start Git with revision::git_in or tests/support/git.rs, which keep a test's Git \
         out of the repository running the tests: {elsewhere:?}"
    );
}
