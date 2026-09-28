//! End-to-end tests of the `jevgate` binary, by command; the shared
//! project helper is here.
mod auth;
mod changes;
mod convention;
mod gallery;
mod gateway;
#[path = "../support/git.rs"]
mod git;
mod hook;
mod manual;
mod mcp;
#[path = "../support/mock_provider.rs"]
mod mock_provider;
mod preview;
mod propose;
mod questions;
mod rules;
mod setup;
#[path = "../support/temp_dir.rs"]
mod temp_dir;
#[cfg(unix)]
mod watch;

use std::process::{Command, Stdio};

struct Project(temp_dir::TempDir);
impl Project {
    fn new() -> Self {
        Self(temp_dir::TempDir::new("jevgate-cli"))
    }
    /// A Git repository with [`JUDGED_RS`] committed as `lib.rs`.
    fn committed() -> Self {
        let project = Self::new();
        std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
        git(&project, &["init", "-q"]);
        git(&project, &["add", "lib.rs"]);
        git(&project, &["commit", "-qm", "start"]);
        project
    }
    /// `jevgate` in the project, with no key or endpoint from the caller's
    /// environment, credentials saved only in the project, and no variable
    /// pointing its Git at the repository running the tests.
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_jevgate"));
        git::isolate(&mut command)
            .current_dir(&self.0)
            .env_remove("TYPESAFE_API_KEY")
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("AI_GATEWAY_API_KEY")
            .env_remove("JEVGATE_BASE_URL")
            .env_remove("CI")
            .env("JEVGATE_CREDENTIAL_STORE", "file")
            .env("JEVGATE_CONFIG_DIR", self.0.join("isolated-auth"));
        command
    }
    /// `jevgate` in the project with `key` as its TypeSafe key, sending its
    /// requests to `provider`.
    fn asking(&self, provider: &mock_provider::MockProvider, key: &str) -> Command {
        let mut command = self.command();
        command
            .env("TYPESAFE_API_KEY", key)
            .env("JEVGATE_BASE_URL", &provider.url);
        command
    }
    /// Runs an offline preview: it succeeds, never prints the key and sends nothing.
    fn preview(&self, args: &[&str]) -> serde_json::Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("do-not-expose"));
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["api_requests"], 0);
        body
    }
    /// `auth status --offline --json` without a credential: exit 2 and an actionable error.
    fn unconfigured_status(&self, extra: &[&str], hint: &str) -> serde_json::Value {
        let output = self
            .command()
            .args(["auth", "status", "--offline", "--json"])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let body: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["configured"], false);
        assert!(body["error"].as_str().unwrap().contains(hint), "{body}");
        body
    }
    fn snapshot(&self) -> Option<serde_json::Value> {
        std::fs::read(self.0.join(".jevgate/latest.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }
    /// `key` saved as `jevgate auth login` saved it before 0.26: bare, in the
    /// owner-only file of the project's isolated configuration directory,
    /// with no provider recorded beside it.
    #[cfg(unix)]
    fn save_credential(&self, key: &str) -> std::path::PathBuf {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let config = self.0.join("isolated-auth");
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&config)
            .unwrap();
        let saved = config.join("credentials");
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&saved)
            .unwrap()
            .write_all(key.as_bytes())
            .unwrap();
        saved
    }
}

/// A function large enough to judge: five body lines.
const JUDGED_RS: &str = "fn f(values: &[i32]) -> i32 {\n    let mut total = 0;\n    for value in values {\n        total += value;\n    }\n    let doubled = total * 2;\n    doubled + 1\n}\n";

/// A function longer than twenty lines, which a split question at the top
/// of its scale makes a function-simplification review.
const LONG_RS: &str = "fn f(values: &[i32]) -> i32 {\n    let mut total = 0;\n    for value in values {\n        total += value;\n    }\n    let mut largest = i32::MIN;\n    for value in values {\n        if *value > largest {\n            largest = *value;\n        }\n    }\n    let mut smallest = i32::MAX;\n    for value in values {\n        if *value < smallest {\n            smallest = *value;\n        }\n    }\n    let spread = largest - smallest;\n    let doubled = total * 2;\n    doubled + spread + 1\n}\n";

/// Run Git in the project, with a fixed identity and no signing, apart from
/// the repository running the tests.
fn git(project: &Project, args: &[&str]) {
    git::run(&project.0, args);
}

/// Run `jevgate hook ARGS`, started by `command`, with `event` on stdin; its
/// reply, its stderr, and that it exited 0.
fn hook(mut command: Command, args: &[&str], event: &str) -> (serde_json::Value, String) {
    use std::io::Write;
    let mut child = command
        .arg("hook")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(event.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    let reply = serde_json::from_slice(&output.stdout).unwrap();
    (reply, stderr)
}

/// A Claude Code event of one session in `project`.
fn hook_event(project: &Project, fields: serde_json::Value) -> String {
    let mut event = fields;
    event["session_id"] = "cli-session".into();
    event["cwd"] = project.0.to_str().unwrap().into();
    event.to_string()
}

/// The stages a dry-run preview plans.
fn stages(preview: &serde_json::Value) -> Vec<String> {
    preview["stages"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

fn dry_run(project: &Project, rules: &[&str]) -> serde_json::Value {
    let mut arguments = vec!["check", "--dry-run", "--format", "json"];
    arguments.extend_from_slice(rules);
    project.preview(&arguments)
}

/// The names of the questions a dry run with `rules` plans to ask, such as
/// `f0_interpreted` for the first function of a pack; none when it plans no
/// request, and the report then leaves `initial_requests` out.
fn asked(project: &Project, rules: &[&str]) -> Vec<String> {
    let mut arguments = vec!["--show-requests"];
    arguments.extend_from_slice(rules);
    dry_run(project, &arguments)["initial_requests"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|request| request["questions"].as_object().unwrap().keys().cloned())
        .collect()
}
