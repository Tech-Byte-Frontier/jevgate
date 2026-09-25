//! Which files can reach another file's members: a caller counts only when it
//! is in the same language family and one of its import lines names the
//! target's module. Matching calls by bare name alone linked unrelated files,
//! such as a Python `update` to a JavaScript `decipher.update`. Java code
//! uses the classes of its own package without importing them, so a Java file
//! also reaches a file of its directory whose class it names.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// Lines that import, load or declare another module.
fn import_lines(source: &str, family: &str) -> Vec<String> {
    source
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
                || ["require ", "require_relative ", "load ", "autoload "]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
                // PHP runs other files with `require_once __DIR__ . '/x.php';`.
                || family == "php"
                    && ["require", "include"].iter().any(|p| line.starts_with(p))
        })
        .map(str::to_string)
        .collect()
}

/// A Java file's directory and the capitalized names its code mentions.
fn java_package(path: &Path, source: &str) -> (PathBuf, BTreeSet<String>) {
    let names = source
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .filter(|word| word.starts_with(|c: char| c.is_ascii_uppercase()))
        .map(str::to_string)
        .collect();
    (path.parent().unwrap_or(Path::new("")).to_path_buf(), names)
}

/// Import and module lines of one file, kept for repeated lookups.
pub struct Imports {
    family: &'static str,
    lines: Vec<String>,
    /// For Java: the file's directory and the capitalized names its code
    /// mentions, the classes of its package it can use without an import.
    package: Option<(PathBuf, BTreeSet<String>)>,
}

impl Imports {
    pub fn new(path: &Path, source: &str) -> Self {
        let family = family(path);
        if family == "csharp" {
            return Self {
                family,
                lines: csharp_lines(source),
                package: None,
            };
        }
        Self {
            family,
            lines: import_lines(source, family),
            package: (family == "java").then(|| java_package(path, source)),
        }
    }

    /// True when these imports name the module that `target` defines. C#
    /// files name no files: a `using` imports a whole namespace, so a file
    /// reaches the class its target is named after (`BasketService.cs`) when
    /// its code names that class or its interface (`IBasketService`).
    pub fn reach(&self, target: &Path) -> bool {
        if self.family.is_empty() || self.family != family(target) {
            return false;
        }
        let mut name = module_name(target);
        if self.family == "csharp" {
            // `Index.cshtml.cs` holds the page model of `Index.cshtml`.
            name = name.split('.').next().unwrap_or("").to_string();
        }
        if name.is_empty() {
            return false;
        }
        if self.family == "csharp" {
            let interface = format!("I{name}");
            return self
                .lines
                .iter()
                .any(|line| names_segment(line, &name) || names_segment(line, &interface));
        }
        let same_package = self.package.as_ref().is_some_and(|(directory, names)| {
            target.parent().unwrap_or(Path::new("")) == directory && names.contains(&name)
        });
        same_package || self.lines.iter().any(|line| names_segment(line, &name))
    }
}

/// The lines of C# code that can name a class: not blank, not a comment and
/// not a `using` or `namespace` line.
fn csharp_lines(source: &str) -> Vec<String> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !["//", "/*", "*", "using ", "namespace "]
                    .iter()
                    .any(|prefix| line.starts_with(prefix))
        })
        .map(str::to_string)
        .collect()
}

fn family(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "rs" => "rust",
        "py" => "python",
        "go" => "go",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" | "phtml" => "php",
        "java" => "java",
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte"
        | "astro" => "javascript",
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
        let cases: [(&str, &str, &[&str], &[&str]); 6] = [
            (
                "app/routes.php",
                "<?php\nuse App\\Application\\Actions\\User\\ListUsersAction;\nrequire_once __DIR__ . '/helpers.php';\n",
                &[
                    "src/Application/Actions/User/ListUsersAction.php",
                    "app/helpers.php",
                ],
                &[
                    "src/Application/Actions/User/ViewUserAction.php",
                    "app/ListUsersAction.py",
                ],
            ),
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
                "use crate::units::{self, compose};\nmod gate;\ninclude!(\"report.rs\");\n",
                &["src/units/mod.rs", "src/gate.rs"],
                &["src/units/questions.rs", "src/report.rs"],
            ),
            (
                "src/Web/Controllers/BasketController.cs",
                "using Shop.Core.Services;\n\n// Uses OrderService indirectly.\npublic class BasketController(IBasketService basket, UriComposer uris) { }\n",
                &[
                    "src/Core/Services/BasketService.cs",
                    "src/Core/UriComposer.cs",
                    "src/Core/IBasketService.cs",
                ],
                &[
                    "src/Core/Services/OrderService.cs",
                    "src/Core/Services/Services.cs",
                    "src/Core/basket.py",
                ],
            ),
            (
                "src/main/java/app/owner/OwnerController.java",
                "package app.owner;\n\nimport app.model.Person;\n\nclass OwnerController {\n    private final OwnerRepository owners;\n}\n",
                &[
                    "src/main/java/app/owner/OwnerRepository.java",
                    "src/main/java/app/model/Person.java",
                ],
                &[
                    "src/main/java/app/owner/PetValidator.java",
                    "src/main/java/app/vet/OwnerRepository.java",
                    "src/main/java/app/owner/owner_repository.py",
                ],
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
