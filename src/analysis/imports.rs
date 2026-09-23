//! Which files can reach another file's members: a caller counts only when it
//! is in the same language family and one of its import lines names the
//! target's module. Matching calls by bare name alone linked unrelated files,
//! such as a Python `update` to a JavaScript `decipher.update`.
use std::path::Path;

/// Import and module lines of one file, kept for repeated lookups.
pub struct Imports {
    family: &'static str,
    lines: Vec<String>,
}

impl Imports {
    pub fn new(path: &Path, source: &str) -> Self {
        let lines = source
            .lines()
            .map(str::trim)
            .filter(|line| {
                [
                    "import ", "from ", "use ", "pub use ", "mod ", "pub mod ", "export ",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix))
                    || line.contains("require(")
                    || line.contains("import(")
            })
            .map(str::to_string)
            .collect();
        Self {
            family: family(path),
            lines,
        }
    }

    /// True when these imports name the module that `target` defines.
    pub fn reach(&self, target: &Path) -> bool {
        if self.family.is_empty() || self.family != family(target) {
            return false;
        }
        let name = module_name(target);
        !name.is_empty() && self.lines.iter().any(|line| names_segment(line, &name))
    }
}

fn family(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "rs" => "rust",
        "py" => "python",
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => "javascript",
        _ => "",
    }
}

/// The name importers use: the file stem, or the directory for index-style files.
fn module_name(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if matches!(stem, "index" | "mod" | "__init__" | "lib" | "main") {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string()
    } else {
        stem.to_string()
    }
}

/// `name` appears as a whole path segment, as in `./name'`, `crate::name::x`,
/// `from .name import` or `import name`.
fn names_segment(line: &str, name: &str) -> bool {
    let separator = |c: char| !(c.is_alphanumeric() || c == '_' || c == '-');
    line.match_indices(name).any(|(start, _)| {
        let before = line[..start].chars().next_back();
        let after = line[start + name.len()..].chars().next();
        before.is_some_and(separator) && after.is_none_or(separator)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callers_need_an_import_of_the_module_in_the_same_language() {
        let cases: [(&str, &str, &[&str], &[&str]); 3] = [
            (
                "src/game/view.ts",
                "import { durationLabel } from './travel-presentation'\nconst x = update()\n",
                &["src/game/travel-presentation.ts"],
                &["src/game/travel.ts", "scripts/travel-presentation.py"],
            ),
            (
                "scripts/sync.py",
                "from project import items\nfrom .fields import canonical_value\n",
                &["scripts/project.py", "scripts/fields.py"],
                &["scripts/project_tools.py"],
            ),
            (
                "src/main.rs",
                "use crate::units::{self, compose};\nmod gate;\n",
                &["src/units/mod.rs", "src/gate.rs"],
                &["src/units/questions.rs"],
            ),
        ];
        for (caller, source, reached, unreached) in cases {
            let imports = Imports::new(Path::new(caller), source);
            for target in reached {
                assert!(imports.reach(Path::new(target)), "{caller} → {target}");
            }
            for target in unreached {
                assert!(!imports.reach(Path::new(target)), "{caller} ↛ {target}");
            }
        }
    }
}
