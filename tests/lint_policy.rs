//! Unused code is deleted and long parameter lists are grouped: no source may
//! suppress those lints, with `allow` or `expect`. Cargo.toml denies them.
use std::{fs, path::Path};

const NEVER_SUPPRESSED: [&str; 4] = ["dead_code", "unused", "too_many_arguments", "complexity"];

fn rust_files(dir: &Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    rust_files(&root.join("tests"), &mut files);
    let mut violations = Vec::new();
    for path in files {
        let source = fs::read_to_string(&path).unwrap();
        for name in suppressed(&source) {
            if NEVER_SUPPRESSED.contains(&name.as_str()) {
                violations.push(format!("{}: {name}", path.display()));
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
