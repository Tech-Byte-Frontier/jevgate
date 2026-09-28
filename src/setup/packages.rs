//! The Claude Code plugin in `plugin/` and the npm launcher in `npm/`,
//! checked against the code: the plugin's hooks are those `init --agent
//! claude` writes, and both carry the crate's version, which pins the plugin
//! users get and the release binary the launcher downloads. After changing the
//! hooks or bumping the version, `JEVGATE_WRITE_PACKAGES=1 cargo test
//! packages` rewrites them. The docs' hooks for setting up by hand are the
//! same, which that leaves to a person.
use super::{
    agents, hooks,
    json::{self, Json},
};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const VERSIONED: [&str; 2] = ["plugin/.claude-plugin/plugin.json", "npm/package.json"];
const PLUGIN_HOOKS: &str = "plugin/hooks/hooks.json";
/// The page whose JSON example after [`BY_HAND`] sets up Claude Code's hooks.
const DOCS: &str = "site/src/coding-agents.md";
const BY_HAND: &str = "By hand, for Claude Code";

/// The plugin's `hooks/hooks.json`: Claude Code's hooks from the same table
/// `init --agent claude` writes into settings.
fn plugin_hooks() -> String {
    let mut document = Json::Object(Vec::new());
    hooks::install(&mut document, &agents::CLAUDE).expect("an empty document takes hooks");
    json::render(&document, &json::Layout::default())
}

/// `text` with `version` set, in the file's own layout.
fn versioned(text: &str, version: &str) -> String {
    let (mut manifest, layout) = json::parse(text).expect("a JSON manifest");
    manifest.set("version", version.into());
    json::render(&manifest, &layout)
}

#[test]
fn the_plugin_and_the_npm_package_match_the_crate() {
    let write = std::env::var_os("JEVGATE_WRITE_PACKAGES").is_some();
    let read = |path: &str| {
        std::fs::read_to_string(format!("{ROOT}/{path}"))
            .unwrap_or_default()
            .replace('\r', "")
    };
    let mut expected: Vec<(&str, String)> = VERSIONED
        .iter()
        .map(|path| (*path, versioned(&read(path), env!("CARGO_PKG_VERSION"))))
        .collect();
    expected.push((PLUGIN_HOOKS, plugin_hooks()));
    let mut stale = Vec::new();
    for (path, text) in expected {
        if write {
            std::fs::write(format!("{ROOT}/{path}"), &text).unwrap();
        }
        if read(path) != text {
            stale.push(path);
        }
    }
    assert!(
        stale.is_empty(),
        "{stale:?} out of date; run JEVGATE_WRITE_PACKAGES=1 cargo test packages"
    );
}

#[test]
fn init_finds_the_plugins_hooks() {
    let (document, _) = json::parse(&plugin_hooks()).unwrap();
    let Some(Json::Object(events)) = document.get("hooks") else {
        panic!("a hooks object");
    };
    let names: Vec<&str> = events.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"]
    );
    assert_eq!(
        hooks::handlers(&document),
        4,
        "`init --agent claude --remove` finds each of them"
    );
}

#[test]
fn the_docs_set_up_by_hand_the_hooks_init_writes() {
    let page = std::fs::read_to_string(format!("{ROOT}/{DOCS}")).unwrap_or_default();
    let example = page
        .split_once(BY_HAND)
        .and_then(|(_, rest)| rest.split_once("```json\n"))
        .and_then(|(_, rest)| rest.split_once("\n```"))
        .map(|(example, _)| example)
        .expect("a JSON example after the by-hand heading");
    let shown: serde_json::Value = serde_json::from_str(example).unwrap();
    let written: serde_json::Value = serde_json::from_str(&plugin_hooks()).unwrap();
    assert_eq!(
        shown, written,
        "{DOCS} sets up other hooks than `init --agent claude` writes"
    );
}
