//! `jevgate init --agent`: each agent's files, JevGate's hooks merged into
//! settings others wrote, running twice, taking out, files JevGate cannot
//! read or did not write, the `PATH` check and the warnings.
use super::{
    agents::{self, Places, Target},
    hooks,
    json::{self, Json},
    *,
};
use crate::tests::Project;

fn places(project: &Project) -> Places {
    Places::under(project.0.join("home"), project.0.join("repo"), |_| None)
}

fn setup(agents: &[Target]) -> AgentSetup {
    AgentSetup {
        agents: agents.to_vec(),
        ..AgentSetup::default()
    }
}

/// Plan and write, as `jevgate init --agent` does without printing.
fn apply(setup: &AgentSetup, places: &Places) -> Plan {
    let plan = Plan::new(setup, places).unwrap();
    plan.apply().unwrap();
    plan
}

fn outcomes(plan: &Plan) -> Vec<Outcome> {
    plan.agents
        .iter()
        .flat_map(|(_, steps)| steps.iter().map(|step| step.outcome))
        .collect()
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// JevGate's handlers in a settings text.
fn jevgates(text: &str) -> usize {
    hooks::handlers(&json::parse(text).unwrap().0)
}

/// Settings another tool wrote, laid out as Claude Code writes them.
const CLAUDE_SETTINGS: &str = r#"{
  "permissions": {
    "allow": [
      "Bash(npm test)"
    ]
  },
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "prettier --write"
          }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "guard"
          }
        ]
      }
    ]
  },
  "model": "opus"
}
"#;

#[test]
fn each_agent_writes_its_own_files_for_the_user_and_for_a_repository() {
    let places = Places::under(PathBuf::from("/h"), PathBuf::from("/r"), |_| None);
    let files = |target, project| -> Vec<PathBuf> {
        agents::files(target, &places, project)
            .into_iter()
            .map(|(path, _)| path)
            .collect()
    };
    let paths = |list: &[&str]| -> Vec<PathBuf> { list.iter().map(PathBuf::from).collect() };
    let cases = [
        (
            Target::Claude,
            false,
            paths(&["/h/.claude/settings.json", "/h/.claude/rules/jevgate.md"]),
        ),
        (
            Target::Claude,
            true,
            paths(&["/r/.claude/settings.json", "/r/.claude/rules/jevgate.md"]),
        ),
        (
            Target::Codex,
            false,
            paths(&["/h/.codex/hooks.json", "/h/.codex/AGENTS.md"]),
        ),
        (
            Target::Codex,
            true,
            paths(&["/r/.codex/hooks.json", "/r/AGENTS.md"]),
        ),
        (
            Target::Gemini,
            false,
            paths(&["/h/.gemini/settings.json", "/h/.gemini/GEMINI.md"]),
        ),
        (
            Target::Gemini,
            true,
            paths(&["/r/.gemini/settings.json", "/r/GEMINI.md"]),
        ),
        (Target::Cursor, false, paths(&["/h/.cursor/hooks.json"])),
        (
            Target::Cursor,
            true,
            paths(&["/r/.cursor/hooks.json", "/r/.cursor/rules/jevgate.mdc"]),
        ),
        (
            Target::Opencode,
            false,
            paths(&[
                "/h/.config/opencode/plugins/jevgate.js",
                "/h/.config/opencode/AGENTS.md",
            ]),
        ),
        (
            Target::Opencode,
            true,
            paths(&["/r/.opencode/plugins/jevgate.js", "/r/AGENTS.md"]),
        ),
    ];
    for (target, project, expected) in cases {
        assert_eq!(
            files(target, project),
            expected,
            "{target:?} project={project}"
        );
    }
    let moved = Places::under(
        PathBuf::from("/h"),
        PathBuf::from("/r"),
        |variable| match variable {
            "CLAUDE_CONFIG_DIR" => Some(PathBuf::from("/claude")),
            "CODEX_HOME" => Some(PathBuf::from("/codex")),
            "XDG_CONFIG_HOME" => Some(PathBuf::from("/config")),
            _ => None,
        },
    );
    assert_eq!(moved.claude, PathBuf::from("/claude"));
    assert_eq!(moved.codex, PathBuf::from("/codex"));
    assert_eq!(moved.opencode, PathBuf::from("/config/opencode"));
    assert_eq!(
        places.show(Path::new("/h/.codex/hooks.json")),
        Path::new("~")
            .join(".codex/hooks.json")
            .display()
            .to_string()
    );
    assert_eq!(places.show(Path::new("/r/AGENTS.md")), "AGENTS.md");
}

#[test]
fn jevgates_hooks_join_other_tools_and_replace_its_own() {
    let claude = Part::Hooks(&agents::CLAUDE);
    // A group shared with another tool, holding an older JevGate handler.
    let shared = CLAUDE_SETTINGS.replace(
        "\"command\": \"prettier --write\"\n          }",
        "\"command\": \"prettier --write\"\n          },\n          {\n            \"type\": \"command\",\n            \"command\": \"/opt/homebrew/bin/jevgate hook\",\n            \"timeout\": 5\n          }",
    );
    assert_eq!(jevgates(&shared), 1);
    let installed = change(Some(&shared), &claude, false).unwrap().unwrap();
    assert_eq!(
        jevgates(&installed),
        4,
        "one handler per event, the old one gone"
    );
    let (settings, _) = json::parse(&installed).unwrap();
    let keys = |value: &Json| match value {
        Json::Object(entries) => entries.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert_eq!(keys(&settings), ["permissions", "hooks", "model"]);
    assert_eq!(
        keys(settings.get("hooks").unwrap()),
        [
            "PostToolUse",
            "PreToolUse",
            "SessionStart",
            "UserPromptSubmit",
            "Stop"
        ]
    );
    assert!(installed.contains("prettier --write") && installed.contains("\"guard\""));
    assert!(!installed.contains("/opt/homebrew"));
    assert_eq!(
        change(Some(&installed), &claude, false).unwrap().as_deref(),
        Some(installed.as_str()),
        "running again changes nothing"
    );
    let original = CLAUDE_SETTINGS;
    let added = change(Some(original), &claude, false).unwrap().unwrap();
    assert_eq!(
        change(Some(&added), &claude, true).unwrap().as_deref(),
        Some(original),
        "taking JevGate's hooks out gives the original back"
    );
    assert_eq!(
        change(Some(original), &claude, true).unwrap().as_deref(),
        Some(original),
        "nothing of JevGate's: untouched"
    );
}

#[test]
fn a_hand_edited_jevgate_hook_is_reset_to_the_defaults() {
    let claude = Part::Hooks(&agents::CLAUDE);
    let installed = change(None, &claude, false).unwrap().unwrap();
    let edited = installed.replace("\"timeout\": 40", "\"timeout\": 5");
    assert_ne!(edited, installed);
    assert_eq!(
        change(Some(&edited), &claude, false).unwrap(),
        Some(installed)
    );
}

#[test]
fn a_file_of_only_jevgates_hooks_is_removed_whole() {
    for set in [&agents::CLAUDE, &agents::CURSOR] {
        let part = Part::Hooks(set);
        let installed = change(None, &part, false).unwrap().unwrap();
        assert_eq!(
            change(Some(&installed), &part, true).unwrap(),
            None,
            "{set:?}"
        );
    }
    assert_eq!(
        change(None, &Part::Hooks(&agents::CLAUDE), true).unwrap(),
        None
    );
}

#[test]
fn each_agent_gets_its_own_hook_fields() {
    let placed = |target| {
        let (path, part) = agents::files(target, &places(&Project::new()), false).remove(0);
        let text = change(None, &part, false).unwrap().unwrap();
        (path, json::parse(&text).unwrap().0)
    };
    let (_, claude) = placed(Target::Claude);
    let edit = &claude.get("hooks").unwrap().get("PostToolUse").unwrap();
    let Json::Array(groups) = edit else { panic!() };
    assert_eq!(
        groups[0].get("matcher"),
        Some(&"Edit|Write|MultiEdit|NotebookEdit".into())
    );
    let handler = match groups[0].get("hooks") {
        Some(Json::Array(handlers)) => handlers[0].clone(),
        _ => panic!("a group of handlers"),
    };
    assert_eq!(
        handler,
        Json::object([
            ("type", "command".into()),
            ("command", "jevgate hook".into()),
            ("timeout", 40u64.into()),
            ("statusMessage", "JevGate is checking the edit".into()),
        ])
    );
    let (_, gemini) = placed(Target::Gemini);
    let rendered = json::render(&gemini, &json::Layout::default());
    assert!(rendered.contains("\"AfterTool\"") && rendered.contains("\"write_file|replace\""));
    assert!(rendered.contains("\"timeout\": 40000") && rendered.contains("\"name\": \"jevgate\""));
    let (_, codex) = placed(Target::Codex);
    assert!(json::render(&codex, &json::Layout::default()).contains("\"apply_patch\""));
    let (_, cursor) = placed(Target::Cursor);
    assert_eq!(cursor.get("version"), Some(&1u64.into()));
    let Some(Json::Array(edits)) = cursor.get("hooks").unwrap().get("postToolUse") else {
        panic!("a list of handlers");
    };
    assert_eq!(
        edits[0],
        Json::object([
            ("command", "jevgate hook --agent cursor".into()),
            ("matcher", "Write".into()),
            ("timeout", 40u64.into()),
        ])
    );
}

#[test]
fn settings_it_cannot_read_whole_are_refused() {
    let claude = Part::Hooks(&agents::CLAUDE);
    for text in [
        "{\n  // mine\n  \"theme\": \"dark\"\n}\n",
        "[]",
        "{\"hooks\": []}",
        "{\"hooks\": {\"Stop\": {}}}",
    ] {
        assert!(change(Some(text), &claude, false).is_err(), "{text}");
    }
}

#[test]
fn jevgates_handlers_are_found_however_written() {
    let shell =
        |command: &str| Json::object([("type", "command".into()), ("command", command.into())]);
    let exec = |command: &str, first: &str| {
        Json::object([
            ("command", command.into()),
            ("args", Json::Array(vec![first.into()])),
        ])
    };
    for (handler, ours) in [
        (shell("jevgate hook"), true),
        (shell("jevgate hook --agent cursor"), true),
        (shell("/usr/local/bin/jevgate hook --timeout 20"), true),
        (
            shell("\"C:\\Program Files\\JevGate\\jevgate.exe\" hook"),
            true,
        ),
        (shell("jevgate.EXE hook"), true),
        (shell("jevgate hook || echo '{}'"), true),
        (exec("jevgate", "hook"), true),
        (exec("jevgate", "check"), false),
        (shell("jevgate check --base HEAD"), false),
        (shell("echo jevgate hook"), false),
        (shell("jevgate hooks"), false),
        (shell("not-jevgate hook"), false),
        (Json::object([("type", "prompt".into())]), false),
    ] {
        assert_eq!(hooks::is_jevgate(&handler), ours, "{handler:?}");
    }
}

#[test]
fn running_again_writes_nothing_and_remove_gives_every_file_back() {
    let project = Project::new();
    let places = places(&project);
    let existing = [
        (places.claude.join("settings.json"), CLAUDE_SETTINGS),
        (places.codex.join("AGENTS.md"), "# My rules\n\nBe brief.\n"),
        (
            places.gemini.join("settings.json"),
            "{\n  \"theme\": \"dark\"\n}\n",
        ),
    ];
    for (path, text) in &existing {
        write(path, text);
    }
    let every = [
        Target::Claude,
        Target::Codex,
        Target::Gemini,
        Target::Cursor,
        Target::Opencode,
    ];
    let first = apply(&setup(&every), &places);
    use Outcome::*;
    assert_eq!(
        outcomes(&first),
        [
            Update, Create, Create, Update, Update, Create, Create, Create, Create
        ]
    );
    let again = Plan::new(&setup(&every), &places).unwrap();
    assert!(
        outcomes(&again).iter().all(|o| *o == Unchanged),
        "{:?}",
        outcomes(&again)
    );
    let removal = AgentSetup {
        remove: true,
        ..setup(&every)
    };
    let removed = apply(&removal, &places);
    assert_eq!(
        outcomes(&removed),
        [
            Update, Remove, Remove, Update, Update, Remove, Remove, Remove, Remove
        ]
    );
    for (path, text) in &existing {
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            *text,
            "{}",
            path.display()
        );
    }
    for target in every {
        for (path, _) in agents::files(target, &places, false) {
            assert!(
                existing.iter().any(|(kept, _)| *kept == path) || !path.exists(),
                "{} is gone",
                path.display()
            );
        }
    }
}

#[test]
fn a_file_it_cannot_read_stops_the_run_before_any_write() {
    let project = Project::new();
    let places = places(&project);
    write(
        &places.gemini.join("settings.json"),
        "{\n  // mine\n  \"theme\": \"dark\"\n}\n",
    );
    let error = Plan::new(&setup(&[Target::Claude, Target::Gemini]), &places)
        .err()
        .unwrap();
    let message = format!("{error:#}");
    assert!(
        message.contains("Cannot set up Gemini CLI in ~")
            && message.contains("nothing was written")
            && message.contains("not plain JSON"),
        "{message}"
    );
    assert!(!places.claude.exists());
}

#[test]
fn a_shared_agents_md_gets_one_block() {
    let project = Project::new();
    let places = places(&project);
    write(&places.root.join("AGENTS.md"), "# Repository\n");
    let both = AgentSetup {
        project: true,
        ..setup(&[Target::Codex, Target::Opencode])
    };
    let plan = apply(&both, &places);
    let text = fs::read_to_string(places.root.join("AGENTS.md")).unwrap();
    assert_eq!(text.matches("<!-- jevgate:begin").count(), 1);
    assert!(text.starts_with("# Repository\n\n<!-- jevgate:begin"));
    let opencode = &plan.agents[1].1;
    assert_eq!(opencode[1].outcome, Outcome::Unchanged);
}

#[test]
fn a_file_jevgate_did_not_write_is_left_alone() {
    let project = Project::new();
    let places = places(&project);
    let mine = places.root.join(".claude/rules/jevgate.md");
    write(&mine, "# My notes on JevGate\n");
    let claude = |remove| AgentSetup {
        project: true,
        remove,
        ..setup(&[Target::Claude])
    };
    let error = Plan::new(&claude(false), &places).err().unwrap();
    assert!(
        format!("{error:#}").contains("not written by JevGate"),
        "{error:#}"
    );
    let removal = apply(&claude(true), &places);
    assert_eq!(outcomes(&removal), [Outcome::Absent, Outcome::Unchanged]);
    assert_eq!(
        fs::read_to_string(&mine).unwrap(),
        "# My notes on JevGate\n"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_settings_file_is_written_through_and_kept() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    let places = places(&project);
    let dotfile = project.0.join("dotfiles/claude.json");
    write(&dotfile, "{}\n");
    fs::set_permissions(&dotfile, fs::Permissions::from_mode(0o640)).unwrap();
    fs::create_dir_all(&places.claude).unwrap();
    let settings = places.claude.join("settings.json");
    std::os::unix::fs::symlink(&dotfile, &settings).unwrap();
    apply(&setup(&[Target::Claude]), &places);
    assert!(settings.is_symlink());
    assert_eq!(jevgates(&fs::read_to_string(&dotfile).unwrap()), 4);
    assert_eq!(
        fs::metadata(&dotfile).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let removal = AgentSetup {
        remove: true,
        ..setup(&[Target::Claude])
    };
    apply(&removal, &places);
    assert!(settings.is_symlink(), "the link is someone's dotfiles");
    assert_eq!(fs::read_to_string(&dotfile).unwrap(), "{}\n");
}

#[cfg(unix)]
#[test]
fn a_repository_cannot_lead_its_files_outside_itself() {
    let project = Project::new();
    let places = places(&project);
    let outside = project.0.join("home/.bashrc");
    write(&outside, "export PATH=$HOME/bin:$PATH\n");
    fs::create_dir_all(&places.root).unwrap();
    std::os::unix::fs::symlink(&outside, places.root.join("AGENTS.md")).unwrap();
    let elsewhere = project.0.join("home/config");
    fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, places.root.join(".cursor")).unwrap();
    for target in [Target::Codex, Target::Cursor] {
        let repository = AgentSetup {
            project: true,
            ..setup(&[target])
        };
        let error = Plan::new(&repository, &places).err().unwrap();
        assert!(
            format!("{error:#}").contains("outside the repository"),
            "{error:#}"
        );
    }
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "export PATH=$HOME/bin:$PATH\n"
    );
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    // A link inside the repository is followed, as a person's own dotfiles are.
    fs::remove_file(places.root.join("AGENTS.md")).unwrap();
    write(&places.root.join("docs/AGENTS.md"), "# Shared\n");
    std::os::unix::fs::symlink("docs/AGENTS.md", places.root.join("AGENTS.md")).unwrap();
    let codex = AgentSetup {
        project: true,
        ..setup(&[Target::Codex])
    };
    apply(&codex, &places);
    assert!(
        fs::read_to_string(places.root.join("docs/AGENTS.md"))
            .unwrap()
            .contains("jevgate:begin")
    );
}

#[test]
fn hooks_running_twice_are_warned_about() {
    let project = Project::new();
    let places = places(&project);
    write(
        &places.claude.join("settings.json"),
        "{\n  \"enabledPlugins\": {\n    \"jevgate@jevgate\": true\n  }\n}\n",
    );
    let claude = Plan::new(&setup(&[Target::Claude]), &places).unwrap();
    let warnings = claude.warnings(&places);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("plugin is enabled"), "{warnings:?}");
    let both = apply(&setup(&[Target::Claude, Target::Cursor]), &places);
    assert!(
        both.warnings(&places)
            .iter()
            .any(|w| w.contains("twice in Cursor"))
    );
    let codex = Plan::new(&setup(&[Target::Codex]), &places).unwrap();
    assert!(codex.warnings(&places).is_empty(), "Codex runs neither");
}

#[test]
fn the_project_is_the_git_work_trees_top() {
    let project = Project::new();
    let nested = project.0.join("repo/src/deep");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(project.0.join("repo/.git")).unwrap();
    assert_eq!(project_root(&nested), project.0.join("repo"));
    let outside = project.0.join("elsewhere");
    fs::create_dir_all(&outside).unwrap();
    assert_eq!(project_root(&outside), outside);
}

#[cfg(unix)]
#[test]
fn the_jevgate_on_path_must_answer_the_hooks() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::new();
    let program = |dir: &str, script: &str| {
        let bin = project.0.join(dir);
        let path = bin.join("jevgate");
        write(&path, &format!("#!/bin/sh\ncat > /dev/null\n{script}"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        bin
    };
    let answers = program("current", "printf '{}\\n'\n");
    assert_eq!(probe::problem(Some(answers.as_os_str())), None);
    let older = program(
        "older",
        "echo \"error: unrecognized subcommand 'hook'\" >&2\nexit 2\n",
    );
    let problem = probe::problem(Some(older.as_os_str())).unwrap();
    assert!(
        problem.contains("exited 2 (error: unrecognized subcommand 'hook')"),
        "{problem}"
    );
    let other = program("other", "echo Approved.\n");
    let problem = probe::problem(Some(other.as_os_str())).unwrap();
    assert!(problem.contains("answered \"Approved.\""), "{problem}");
    let empty = project.0.join("empty");
    fs::create_dir_all(&empty).unwrap();
    write(&empty.join("jevgate"), "not executable");
    let problem = probe::problem(Some(empty.as_os_str())).unwrap();
    assert!(
        problem.starts_with("no jevgate is on your PATH"),
        "{problem}"
    );
    let first = std::env::join_paths([&empty, &answers, &older]).unwrap();
    assert_eq!(
        probe::find(&first, "jevgate"),
        Some(answers.join("jevgate"))
    );
}
