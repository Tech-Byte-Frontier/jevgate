//! SpacetimeDB modules: the packages access control keeps, and the framework
//! version each module's manifest declares.
use super::*;

/// Access control reads application source only for SpacetimeDB modules:
/// alone among the code rules, it keeps just the files of their packages.
pub(super) fn keep_module_packages(args: &CheckArgs, inputs: &mut Vec<Input>) {
    let only_access = args.rules.iter().all(|rule| {
        rule == crate::catalog::ACCESS_CONTROL
            || !crate::catalog::find(rule).is_some_and(|r| args.code_rules_include(r.key))
    });
    if !only_access {
        return;
    }
    let roots: Vec<PathBuf> = inputs
        .iter()
        .filter_map(|i| Some(i.framework.as_ref()?.root.clone()))
        .collect();
    inputs.retain(|input| roots.iter().any(|root| input.result.path.starts_with(root)));
}

/// The package of a SpacetimeDB module file: the nearest `package.json` above
/// it that declares `spacetimedb`. Only the version is read from it, never
/// the manifest's other text.
pub(super) fn spacetimedb_package(root: &Path, relative: &Path) -> Framework {
    for directory in relative.ancestors().skip(1) {
        if let Ok(text) = read_source(&root.join(directory).join("Cargo.toml"), LOCAL_PARSE_MAX)
            && let Ok(table) = text.parse::<toml::Table>()
            && let Some(dependency) = table.get("dependencies").and_then(|d| d.get("spacetimedb"))
        {
            let version = dependency
                .as_str()
                .or_else(|| dependency.get("version")?.as_str())
                .unwrap_or("");
            return Framework {
                root: directory.to_path_buf(),
                version: version.trim_start_matches(['^', '~', '=', 'v', ' ']).into(),
            };
        }
        let manifest = root.join(directory).join("package.json");
        let Ok(text) = read_source(&manifest, LOCAL_PARSE_MAX) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        for section in ["dependencies", "devDependencies", "peerDependencies"] {
            if let Some(version) = json[section]["spacetimedb"].as_str() {
                return Framework {
                    root: directory.to_path_buf(),
                    version: version.trim_start_matches(['^', '~', '=', 'v', ' ']).into(),
                };
            }
        }
    }
    Framework {
        root: relative.parent().unwrap_or(Path::new("")).to_path_buf(),
        version: String::new(),
    }
}
