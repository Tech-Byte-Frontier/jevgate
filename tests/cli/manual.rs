//! Shell completions and man pages.
use super::*;

#[test]
fn completions_cover_every_shell_and_command_offline() {
    let project = Project::new();
    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let output = project
            .command()
            .args(["completions", shell])
            .output()
            .unwrap();
        assert!(output.status.success(), "{shell}");
        let script = String::from_utf8(output.stdout).unwrap();
        // Fish names long options without dashes (`-l base`).
        for word in ["check", "baseline", "base", "include-tests"] {
            assert!(script.contains(word), "{shell} completes {word}");
        }
    }
    assert!(!project.0.join(".jevgate").exists());
}

#[test]
fn man_pages_are_named_by_command_and_unknown_commands_fail() {
    let project = Project::new();
    let page = |arguments: &[&str]| project.command().args(arguments).output().unwrap();
    let root = page(&["man"]);
    assert!(root.status.success());
    assert!(String::from_utf8_lossy(&root.stdout).contains(".TH jevgate 1"));
    let check = page(&["man", "check"]);
    let check = String::from_utf8_lossy(&check.stdout);
    assert!(check.contains(".TH jevgate-check 1"), "{check}");
    assert!(check.contains(env!("CARGO_PKG_VERSION")));
    assert!(check.contains("Reading the JSON report"));
    for unknown in ["nope", "help"] {
        let output = page(&["man", unknown]);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("Unknown command"));
    }
}
