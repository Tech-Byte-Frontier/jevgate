mod copies;

pub use copies::{generated_header, generated_source, vendored};

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
            || (name.ends_with(".cs") && components.iter().any(|c| dotnet_test_project(c)))
            || test_name(&name)
            || name.ends_with(".rb") && under(&["spec", "step_definitions"])
            || phpunit_name(path)
        {
            "test"
        } else {
            "source"
        }
    }
}

/// File names generators use, such as `api.generated.ts` or `bundle.min.js`,
/// and the C# files that designers and source generators write
/// (`Form1.Designer.cs`, `App.g.cs`).
fn generated_name(name: &str) -> bool {
    name.contains(".generated.")
        || name.contains(".gen.")
        || name.ends_with(".min.js")
        || name == "database.types.ts"
        || name.ends_with(".designer.cs")
        || name.ends_with(".g.cs")
        || name.ends_with(".g.i.cs")
}

/// A .NET test project directory, named by convention after the project it
/// tests: `Shop.Tests`, `Shop.UnitTests`, `Shop.IntegrationTests`. Only its
/// C# files are tests by this name.
fn dotnet_test_project(directory: &str) -> bool {
    [
        ".tests",
        ".test",
        ".unittests",
        ".integrationtests",
        ".functionaltests",
        ".specs",
    ]
    .iter()
    .any(|suffix| directory.len() > suffix.len() && directory.ends_with(suffix))
}

/// Test file naming conventions across the supported languages. Cargo and
/// Go decide which files are tests, so a `test_*.rs` or `test_*.go` file is
/// ordinary code, such as a module that locates tests. RSpec runs
/// `*_spec.rb`, and a Ruby project keeps them and their support under
/// `spec/` (Cucumber steps under `step_definitions/`).
fn test_name(name: &str) -> bool {
    (name.starts_with("test_") && !name.ends_with(".rs") && !name.ends_with(".go"))
        || name.contains(".test.")
        || name.contains(".spec.")
        || name.contains("_test.")
        || name.ends_with("_tests.rs")
        || name == "tests.rs"
        || name.ends_with("_spec.rb")
}

/// PHPUnit's convention, `UserTest.php`; the case keeps `latest.php` apart.
fn phpunit_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix("Test.php"))
        .is_some_and(|stem| !stem.is_empty())
}

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
        "kts", "scala", "c", "h", "cpp", "cc", "cxx", "hpp", "cs", "rb", "php", "phtml", "swift",
        "dart", "lua", "ex", "exs", "zig", "sh", "vue", "svelte", "astro", "sql",
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
        assert_eq!(classifier.role(Path::new("src/OrderTest.php")), "test");
        assert_eq!(classifier.role(Path::new("src/latest.php")), "source");
        assert_eq!(classifier.role(Path::new("src/Test.php")), "source");
        assert_eq!(
            classifier.role(Path::new("src/test_locations.rs")),
            "source"
        );
        assert_eq!(classifier.role(Path::new("pkg/test_helpers.go")), "source");
        assert_eq!(classifier.role(Path::new("src/orders.test.ts")), "test");
        assert_eq!(classifier.role(Path::new("pkg/orders_test.go")), "test");
        assert_eq!(classifier.role(Path::new("lib/orders_spec.rb")), "test");
        assert_eq!(classifier.role(Path::new("spec/support/models.rb")), "test");
        assert_eq!(
            classifier.role(Path::new("features/step_definitions/steps.rb")),
            "test"
        );
        assert_eq!(
            classifier.role(Path::new("spec/fixtures/app.rb")),
            "fixture"
        );
        assert_eq!(classifier.role(Path::new("lib/spec/openapi.ts")), "source");
    }

    #[test]
    fn dotnet_test_projects_are_tests_and_designer_files_generated() {
        let classifier = super::Classifier::new(&Default::default()).unwrap();
        for path in [
            "src/Shop.Tests/BasketTests.cs",
            "Shop.UnitTests/Services/Orders.cs",
            "src/Shop.IntegrationTests/Fixture.cs",
        ] {
            assert_eq!(classifier.role(Path::new(path)), "test", "{path}");
        }
        assert_eq!(classifier.role(Path::new("src/Tests/Api.cs")), "test");
        assert_eq!(classifier.role(Path::new("src/Shop/Tests.cs")), "source");
        assert_eq!(
            classifier.role(Path::new("src/Shop.Web/Orders.cs")),
            "source"
        );
        for path in [
            "Forms/Main.Designer.cs",
            "obj/App.g.cs",
            "Views/Page.g.i.cs",
        ] {
            assert_eq!(classifier.role(Path::new(path)), "generated", "{path}");
        }
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
        let client = "/* tslint:disable */\n/* eslint-disable */\n/**\n * Pets API\n * No description provided\n *\n * The version of the OpenAPI document: 1.0.0\n *\n *\n * NOTE: This class is auto generated by OpenAPI Generator.\n * Do not edit the class manually.\n */\nexport class PetsApi {}\n";
        assert!(generated_header(client), "a marker after a title block");
    }
}
