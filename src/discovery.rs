use crate::{boundary::globs, config::Config};
use anyhow::Result;
use globset::GlobSet;
use std::path::Path;

pub const SKIPPED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    "venv",
    "__pycache__",
    "coverage",
];
pub struct Classifier {
    generated: GlobSet,
    tests: GlobSet,
}
impl Classifier {
    pub fn new(config: &Config) -> Result<Self> {
        Ok(Self {
            generated: globs(&config.generated)?,
            tests: globs(&config.tests)?,
        })
    }
    pub fn role(&self, path: &Path) -> &'static str {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let components: Vec<_> = path
            .iter()
            .map(|s| s.to_string_lossy().to_lowercase())
            .collect();
        let under = |dirs: &[&str]| components.iter().any(|c| dirs.contains(&c.as_str()));
        if script_extension(path) {
            "script"
        } else if self.generated.is_match(path) || generated_name(&name) {
            "generated"
        } else if name.ends_with(".d.ts") || name.ends_with(".d.mts") || name.ends_with(".d.cts") {
            "declarations"
        } else if under(&["fixtures", "__fixtures__", "__snapshots__", "testdata"]) {
            "fixture"
        } else if under(&["migrations"]) {
            "migration"
        } else if self.tests.is_match(path)
            || under(&["test", "tests", "__tests__"])
            || test_name(&name)
        {
            "test"
        } else {
            "source"
        }
    }
}

/// File names generators use, such as `api.generated.ts` or `bundle.min.js`.
fn generated_name(name: &str) -> bool {
    name.contains(".generated.")
        || name.contains(".gen.")
        || name.ends_with(".min.js")
        || name == "database.types.ts"
}

/// Test file naming conventions across the supported languages. Cargo and
/// Go decide which files are tests, so a `test_*.rs` or `test_*.go` file is
/// ordinary code, such as a module that locates tests.
fn test_name(name: &str) -> bool {
    (name.starts_with("test_") && !name.ends_with(".rs") && !name.ends_with(".go"))
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.contains("_test.")
        || name.ends_with("_tests.rs")
        || name == "tests.rs"
}

/// A third-party library copied into the repository: a release file named
/// with its version (`jquery-3.6.0.js`, `editor-6.0.1.bundle.js`), the readable
/// build beside a minified one of the same name (`vue.js` and `vue.min.js`), or
/// a leading preserved license banner (`/*!` or `@license`) that names a
/// version. `path` is on disk, for the sibling.
pub fn vendored(path: &Path, source: Option<&str>) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let script = ["js", "mjs", "cjs"].contains(&file_extension(path).as_str());
    let stem = name.split('.').next().unwrap_or("");
    let minified_sibling = script
        && !name.contains(".min.")
        && path
            .with_file_name(format!("{stem}.min.{}", file_extension(path)))
            .is_file();
    (script && versioned_name(&name)) || minified_sibling || source.is_some_and(license_banner)
}

/// A file name that carries a release version, such as `lib-1.2.3.min.js`.
fn versioned_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..bytes.len()).any(|i| {
        matches!(bytes[i - 1], b'-' | b'.' | b'_')
            && name[i..]
                .trim_start_matches('v')
                .split('.')
                .take(3)
                .filter(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .count()
                == 3
    })
}

/// The first comment preserves a library's license and names its version:
/// bundlers keep `/*!` and `@license` comments in the builds they ship.
fn license_banner(source: &str) -> bool {
    let head = source.trim_start();
    let Some(end) = head.strip_prefix("/*").and_then(|rest| rest.find("*/")) else {
        return false;
    };
    let comment = &head[..end + 2];
    (comment.starts_with("/*!") || comment.contains("@license"))
        && comment
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .any(|word| {
                let parts: Vec<&str> = word.trim_matches('.').split('.').collect();
                parts.len() >= 3 && parts.iter().all(|p| !p.is_empty())
            })
}

/// A generated-code marker in the leading comment lines, such as `@generated`,
/// Go's `Code generated ... DO NOT EDIT.` or "automatically generated".
pub fn generated_header(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(5)
        .take_while(|line| COMMENT_STARTS.iter().any(|c| line.starts_with(c)))
        .any(|line| {
            let line = line.to_ascii_lowercase();
            GENERATED_MARKERS.iter().any(|marker| line.contains(marker))
        })
}

/// Output of a bundler, minifier or compiler rather than source a person
/// edits: a generated-code header, a trailing source map reference, or text
/// whose lines are almost all longer than people write them.
pub fn generated_source(source: &str) -> bool {
    generated_header(source) || source_map_reference(source) || minified(source)
}

/// Compilers and bundlers end their output with `//# sourceMappingURL=…`.
fn source_map_reference(source: &str) -> bool {
    source
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .is_some_and(|line| {
            line.starts_with("//# sourceMappingURL=") || line.starts_with("/*# sourceMappingURL=")
        })
}

/// A line at least this long is not one a person wrote.
const MINIFIED_LINE_BYTES: usize = 1000;

/// Nine tenths of a file of at least `MINIFIED_LINE_BYTES` in lines of that
/// length: minified bundles are one or a few such lines. A long data line in
/// hand-written source leaves the rest of the file below that share.
fn minified(source: &str) -> bool {
    let long: usize = source
        .lines()
        .filter(|line| line.len() >= MINIFIED_LINE_BYTES)
        .map(str::len)
        .sum();
    source.len() >= MINIFIED_LINE_BYTES && long * 10 >= source.len() * 9
}

const COMMENT_STARTS: &[&str] = &["//", "#", "/*", "*", "<!--", "--", "\"\"\""];
const GENERATED_MARKERS: &[&str] = &[
    "@generated",
    "do not edit",
    "automatically generated",
    "auto-generated",
    "autogenerated",
    "code generated by",
];

fn file_extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn script_extension(path: &Path) -> bool {
    matches!(
        file_extension(path).as_str(),
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "ps1" | "bat" | "cmd"
    )
}

pub fn source(path: &Path, extra: &[String]) -> bool {
    let extension = file_extension(path);
    [
        "rs", "py", "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "go", "java", "kt",
        "kts", "scala", "c", "h", "cpp", "cc", "cxx", "hpp", "cs", "rb", "php", "swift", "dart",
        "lua", "ex", "exs", "zig", "sh", "vue", "svelte", "astro", "sql",
    ]
    .contains(&extension.as_str())
        || extra.contains(&extension)
}

#[cfg(test)]
mod tests {
    use super::{generated_header, generated_source};
    use std::path::Path;

    #[test]
    fn test_prefixed_names_are_tests_where_the_toolchain_does_not_decide() {
        let classifier = super::Classifier::new(&Default::default()).unwrap();
        assert_eq!(classifier.role(Path::new("app/test_orders.py")), "test");
        assert_eq!(classifier.role(Path::new("src/test_orders.js")), "test");
        assert_eq!(
            classifier.role(Path::new("src/test_locations.rs")),
            "source"
        );
        assert_eq!(classifier.role(Path::new("pkg/test_helpers.go")), "source");
        assert_eq!(classifier.role(Path::new("src/orders.test.ts")), "test");
        assert_eq!(classifier.role(Path::new("pkg/orders_test.go")), "test");
    }

    #[test]
    fn copied_libraries_are_vendored() {
        let dir = crate::tests::Project::new();
        dir.write("static/vue.js", "var a = 1;\n");
        dir.write("static/vue.min.js", "var a=1;\n");
        dir.write("static/app.js", "var a = 1;\n");
        let at = |name: &str| dir.0.join(name);
        assert!(super::vendored(&at("static/jquery-3.6.0.js"), None));
        assert!(super::vendored(
            &at("static/cm-editor-6.0.1.bundle.js"),
            None
        ));
        assert!(
            super::vendored(&at("static/vue.js"), None),
            "readable build beside a minified one"
        );
        assert!(!super::vendored(&at("static/app.js"), None));
        assert!(!super::vendored(&at("src/migration-2024.ts"), None));
        let banner =
            "/*!\n * Chart helpers v2.6.8\n * Released under the MIT License.\n */\nvar a = 1;\n";
        assert!(super::vendored(&at("lib/chart.js"), Some(banner)));
        let own = "/*!\n * Our dashboard. Copyright 2024.\n */\nvar a = 1;\n";
        assert!(
            !super::vendored(&at("lib/dashboard.js"), Some(own)),
            "no version"
        );
    }

    #[test]
    fn minified_bundles_and_compiled_output_are_generated() {
        let bundle = format!(
            "const{{a:e}}=globalThis;{}\n",
            "function t(n){return n+1}".repeat(80)
        );
        assert!(generated_source(&bundle));
        assert!(generated_source(
            "\"use strict\";\nexports.x = 1;\n//# sourceMappingURL=index.js.map\n"
        ));
        let data = format!(
            "{}\nconst LOGO = \"{}\";\n",
            "fn main() {}\n".repeat(200),
            "A".repeat(1200)
        );
        assert!(!generated_source(&data));
        assert!(!generated_source("fn main() {}\n"));
    }

    #[test]
    fn generated_headers_are_recognized_only_in_leading_comments() {
        assert!(generated_header(
            "// THIS FILE IS AUTOMATICALLY GENERATED BY SPACETIMEDB.\nexport {}\n"
        ));
        assert!(generated_header(
            "// Code generated by protoc-gen-go. DO NOT EDIT.\npackage x\n"
        ));
        assert!(generated_header(
            "#!/usr/bin/env python\n# @generated by tool\n"
        ));
        assert!(!generated_header(
            "export const note = 'do not edit'\n// @generated\n"
        ));
        assert!(!generated_header("// Utilities\nfn main() {}\n"));
    }
}
