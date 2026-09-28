//! End-to-end tests of the `jevgate` binary, by command; the shared
//! project helper is here.
mod auth;
mod changes;
mod gateway;
#[path = "../support/git.rs"]
mod git;
mod manual;
mod mcp;
#[path = "../support/mock_provider.rs"]
mod mock_provider;
mod preview;
mod rules;
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

/// Run Git in the project, with a fixed identity and no signing, apart from
/// the repository running the tests.
fn git(project: &Project, args: &[&str]) {
    git::run(&project.0, args);
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
