use crate::config::{Config, globs};
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
        if script_extension(path) {
            "script"
        } else if self.generated.is_match(path)
            || name.contains(".generated.")
            || name.contains(".gen.")
            || name.ends_with(".min.js")
            || name == "database.types.ts"
        {
            "generated"
        } else if name.ends_with(".d.ts") || name.ends_with(".d.mts") || name.ends_with(".d.cts") {
            "declarations"
        } else if components.iter().any(|c| {
            ["fixtures", "__fixtures__", "__snapshots__", "testdata"].contains(&c.as_str())
        }) {
            "fixture"
        } else if components.iter().any(|c| c == "migrations") {
            "migration"
        } else if self.tests.is_match(path)
            || components
                .iter()
                .any(|c| ["test", "tests", "__tests__"].contains(&c.as_str()))
            || name.starts_with("test_")
            || name.contains(".test.")
            || name.contains(".spec.")
            || name.contains("_test.")
            || name.ends_with("_tests.rs")
            || name == "tests.rs"
        {
            "test"
        } else {
            "source"
        }
    }
}

fn script_extension(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "ps1" | "bat" | "cmd"
    )
}

pub fn source(path: &Path, extra: &[String]) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    [
        "rs", "py", "js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "go", "java", "kt",
        "kts", "scala", "c", "h", "cpp", "cc", "cxx", "hpp", "cs", "rb", "php", "swift", "dart",
        "lua", "ex", "exs", "zig", "sh", "vue", "svelte", "sql",
    ]
    .contains(&extension.as_str())
        || extra.contains(&extension)
}
