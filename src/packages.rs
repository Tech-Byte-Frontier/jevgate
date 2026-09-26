//! The package each source file belongs to, from the nearest manifest above
//! it, and which packages can share code: copies in packages with no local
//! dependency path between them are separate projects, such as starter
//! templates or example apps kept side by side, not one codebase.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// A package: the directory of its manifest, its declared name and the
/// names of the packages it depends on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Package {
    pub dir: PathBuf,
    pub name: Option<String>,
    pub dependencies: BTreeSet<String>,
}

const MANIFEST_BYTES: u64 = 1_048_576;

/// The package of a file: the nearest directory at or above it, up to the
/// root, with a `package.json`, `Cargo.toml`, `pyproject.toml` or `go.mod`.
pub fn package(root: &Path, relative: &Path) -> Option<Package> {
    relative.ancestors().skip(1).find_map(|dir| {
        let read = |name: &str| {
            let path = root.join(dir).join(name);
            let small =
                std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= MANIFEST_BYTES);
            small.then(|| std::fs::read_to_string(path).ok()).flatten()
        };
        let manifests = [
            read("package.json").map(|t| node_manifest(&t)),
            read("Cargo.toml").map(|t| cargo_manifest(&t)),
            read("pyproject.toml").map(|t| python_manifest(&t)),
            read("go.mod").map(|t| go_manifest(&t)),
        ];
        let mut package = Package {
            dir: dir.to_path_buf(),
            ..Default::default()
        };
        let mut found = false;
        for (name, dependencies) in manifests.into_iter().flatten() {
            found = true;
            package.name = package.name.or(name);
            package.dependencies.extend(dependencies);
        }
        found.then_some(package)
    })
}

/// A manifest's declared name and dependency names.
type Manifest = (Option<String>, Vec<String>);

fn node_manifest(text: &str) -> Manifest {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return (None, Vec::new());
    };
    let dependencies = ["dependencies", "devDependencies", "peerDependencies"]
        .iter()
        .filter_map(|section| json[section].as_object())
        .flat_map(|names| names.keys().cloned())
        .collect();
    (json["name"].as_str().map(str::to_string), dependencies)
}

fn cargo_manifest(text: &str) -> Manifest {
    let Ok(table) = text.parse::<toml::Table>() else {
        return (None, Vec::new());
    };
    let name = table
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    let dependencies = ["dependencies", "dev-dependencies", "build-dependencies"]
        .iter()
        .filter_map(|section| table.get(*section).and_then(toml::Value::as_table))
        .flat_map(|names| names.keys().cloned())
        .collect();
    (name, dependencies)
}

/// A Go module's path and the modules it requires, on `require` lines and
/// in `require ( … )` blocks. Without it, Online Boutique's Go services,
/// each its own module, read as one package, and 9 of 10 copies found
/// between them were wrong: each service is built on its own.
fn go_manifest(text: &str) -> Manifest {
    let lines: Vec<&str> = text
        .lines()
        .map(|line| line.split("//").next().unwrap_or("").trim())
        .collect();
    let name = lines
        .iter()
        .find_map(|line| line.strip_prefix("module "))
        .map(|path| path.trim_matches('"').to_string());
    (name, go_requirements(&lines))
}

/// The module paths of `require` lines and of `require ( … )` blocks.
fn go_requirements(lines: &[&str]) -> Vec<String> {
    let mut required = Vec::new();
    let mut block = false;
    for line in lines {
        let requirement = match (block, *line) {
            (true, ")") => {
                block = false;
                None
            }
            (true, entry) => Some(entry),
            (false, line) if line.starts_with("require (") => {
                block = true;
                None
            }
            (false, line) => line.strip_prefix("require "),
        };
        required.extend(
            requirement
                .and_then(|r| r.split_whitespace().next())
                .map(str::to_string),
        );
    }
    required
}

fn python_manifest(text: &str) -> Manifest {
    let Ok(table) = text.parse::<toml::Table>() else {
        return (None, Vec::new());
    };
    let project = table.get("project");
    let name = project
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .map(str::to_string);
    let dependencies = project
        .and_then(|p| p.get("dependencies"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(toml::Value::as_str)
        .map(requirement_name)
        .collect();
    (name, dependencies)
}

/// The distribution name at the start of a Python requirement such as
/// `shared-models>=1.2`.
fn requirement_name(requirement: &str) -> String {
    requirement
        .split(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
        .next()
        .unwrap_or("")
        .to_string()
}

/// Whether code in two packages can share one implementation: the same or
/// nested packages, a file outside any package, one depending on the other,
/// or both depending on a third package of this repository (`local`).
pub fn linked(a: Option<&Package>, b: Option<&Package>, local: &BTreeSet<String>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return true;
    };
    let named = |p: &Package, other: &Package| {
        other
            .name
            .as_ref()
            .is_some_and(|n| p.dependencies.contains(n))
    };
    a.dir.starts_with(&b.dir)
        || b.dir.starts_with(&a.dir)
        || named(a, b)
        || named(b, a)
        || a.dependencies
            .intersection(&b.dependencies)
            .any(|name| local.contains(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packages_share_code_only_through_local_dependencies() {
        let project = crate::tests::Project::new();
        project.write("package.json", r#"{"name": "root", "private": true}"#);
        project.write(
            "apps/web/package.json",
            r#"{"name": "web", "dependencies": {"shared": "workspace:*", "react": "19"}}"#,
        );
        project.write(
            "apps/api/package.json",
            r#"{"name": "api", "dependencies": {"shared": "workspace:*"}}"#,
        );
        project.write("packages/shared/package.json", r#"{"name": "shared"}"#);
        project.write(
            "templates/a/package.json",
            r#"{"name": "a", "dependencies": {"react": "19"}}"#,
        );
        project.write(
            "templates/b/Cargo.toml",
            "[package]\nname = \"b\"\n[dependencies]\nserde = \"1\"\n",
        );
        let at = |path: &str| package(&project.0, Path::new(path));
        let web = at("apps/web/src/App.tsx");
        let api = at("apps/api/src/index.ts");
        let shared = at("packages/shared/src/lib.ts");
        let template = at("templates/a/src/App.tsx");
        let rust = at("templates/b/src/main.rs");
        let local: BTreeSet<String> = [&web, &api, &shared, &template, &rust]
            .iter()
            .filter_map(|p| p.as_ref()?.name.clone())
            .chain(["root".to_string()])
            .collect();
        assert_eq!(web.as_ref().unwrap().dir, Path::new("apps/web"));
        assert_eq!(rust.as_ref().unwrap().name.as_deref(), Some("b"));
        assert!(linked(web.as_ref(), api.as_ref(), &local));
        assert!(linked(web.as_ref(), shared.as_ref(), &local));
        assert!(!linked(web.as_ref(), template.as_ref(), &local));
        assert!(!linked(template.as_ref(), rust.as_ref(), &local));
        // Go modules: separate services, and two that share a local module.
        project.write(
            "src/frontend/go.mod",
            "module example.com/shop/frontend // the web tier\n\ngo 1.22\n\nrequire (\n\tgithub.com/gorilla/mux v1.8.1\n\texample.com/shop/common v0.0.0\n)\n",
        );
        project.write(
            "src/checkout/go.mod",
            "module example.com/shop/checkout\n\nrequire example.com/shop/common v0.0.0\n",
        );
        project.write(
            "src/shipping/go.mod",
            "module example.com/shop/shipping\n\nrequire github.com/gorilla/mux v1.8.1\n",
        );
        project.write("src/common/go.mod", "module example.com/shop/common\n");
        let frontend = at("src/frontend/main.go");
        let checkout = at("src/checkout/main.go");
        let shipping = at("src/shipping/main.go");
        let common = at("src/common/log.go");
        let local: BTreeSet<String> = [&frontend, &checkout, &shipping, &common]
            .iter()
            .filter_map(|p| p.as_ref()?.name.clone())
            .collect();
        assert_eq!(
            frontend.as_ref().unwrap().name.as_deref(),
            Some("example.com/shop/frontend")
        );
        assert!(linked(frontend.as_ref(), checkout.as_ref(), &local));
        assert!(!linked(frontend.as_ref(), shipping.as_ref(), &local));
        assert!(linked(
            at("scripts/x.ts").as_ref(),
            template.as_ref(),
            &local
        ));
        assert!(linked(None, web.as_ref(), &local));
    }
}
