//! The coding agents `jevgate init --agent` sets up: where each keeps its
//! hooks and instructions, for one user or one repository, and the hooks it
//! gets. The timeouts sit above the hook's own budgets (10 s at a session or
//! turn start, 30 s after an edit, 50 s at the end of a turn), so the hook
//! gives up and answers before the agent stops waiting for it.
use super::{
    hooks::{Hook, HookSet, Shape},
    text,
};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// A coding agent `jevgate init --agent` can set up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Target {
    /// Claude Code: hooks in settings.json, instructions in rules/jevgate.md
    Claude,
    /// Codex: hooks.json, and a block in AGENTS.md
    Codex,
    /// Cursor: hooks.json, and rules/jevgate.mdc in a repository
    Cursor,
    /// Gemini CLI: hooks in settings.json, and a block in GEMINI.md
    Gemini,
    /// OpenCode 1.x: a plugin running jevgate hook, and a block in AGENTS.md
    Opencode,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Cursor => "Cursor",
            Self::Gemini => "Gemini CLI",
            Self::Opencode => "OpenCode",
        }
    }
}

/// The commands the hooks run. Each must stay byte for byte the same across
/// versions: Codex trusts a hook by its hash and Gemini CLI by its name and
/// command, and both would ask again after any change.
///
/// A `jevgate` missing from the agent's `PATH` exits 127 in the shell, and
/// one older than 0.27 exits 2 on `hook`, which Claude Code reads as "erase
/// the prompt" and Codex, with no cap on stop blocks, as "keep working"
/// until someone interrupts it. `init` checks the `jevgate` on its own
/// `PATH`, but not the one a teammate has, nor the one an agent finds from a
/// login shell (Codex runs `$SHELL -lc`). So where the agent's shell allows,
/// a failure answers the agent itself: Claude Code runs hooks with `sh`, Git
/// Bash (which Git for Windows brings, and the hook needs Git) or PowerShell
/// 7, and Codex with the login shell, all of which read `||`.
pub(super) const COMMAND: &str = "jevgate hook || echo '{\"systemMessage\": \"JevGate could not check: jevgate hook is missing, older than 0.27 or failed. Install JevGate 0.27 or later where this agent finds it (https://tech-byte-frontier.github.io/jevgate/install.html). Nothing was blocked.\"}'";
/// Gemini CLI runs hooks with `bash -c`, or with Windows PowerShell 5.1,
/// which has no `||`. It denies on any exit other than 0 and 1, reading
/// stderr as the reason when stdout is empty (hookRunner.js, 0.61.0), so a
/// missing `jevgate` would block every prompt. Exiting 0 turns the shell's
/// error into a message Gemini CLI shows the person, in both shells.
pub(super) const GEMINI_COMMAND: &str = "jevgate hook; exit 0";
/// Cursor's shell is not documented, so its command is left plain; Cursor
/// lets any exit but 2 through.
const CURSOR_COMMAND: &str = "jevgate hook --agent cursor";
const START_SECS: u64 = 20;
const EDIT_SECS: u64 = 40;
const STOP_SECS: u64 = 60;
const CHECKING_EDIT: &str = "JevGate is checking the edit";
const CHECKING_TURN: &str = "JevGate is checking this turn";

const fn start(event: &'static str) -> Hook {
    Hook {
        event,
        matcher: None,
        timeout: START_SECS,
        status: None,
    }
}

const fn edit(event: &'static str, matcher: &'static str) -> Hook {
    Hook {
        event,
        matcher: Some(matcher),
        timeout: EDIT_SECS,
        status: Some(CHECKING_EDIT),
    }
}

const fn stop(event: &'static str) -> Hook {
    Hook {
        event,
        matcher: None,
        timeout: STOP_SECS,
        status: Some(CHECKING_TURN),
    }
}

/// Claude Code's hooks, which the plugin in `plugin/` also installs.
pub(super) const CLAUDE_HOOKS: [Hook; 4] = [
    start("SessionStart"),
    start("UserPromptSubmit"),
    edit("PostToolUse", "Edit|Write|MultiEdit|NotebookEdit"),
    stop("Stop"),
];
pub(super) const CLAUDE: HookSet = HookSet {
    shape: Shape::Claude,
    command: COMMAND,
    hooks: &CLAUDE_HOOKS,
};
const CODEX: HookSet = HookSet {
    shape: Shape::Claude,
    command: COMMAND,
    hooks: &[
        start("SessionStart"),
        start("UserPromptSubmit"),
        edit("PostToolUse", "apply_patch"),
        stop("Stop"),
    ],
};
const GEMINI: HookSet = HookSet {
    shape: Shape::Gemini,
    command: GEMINI_COMMAND,
    hooks: &[
        start("SessionStart"),
        start("BeforeAgent"),
        edit("AfterTool", "write_file|replace"),
        stop("AfterAgent"),
    ],
};
pub(super) const CURSOR: HookSet = HookSet {
    shape: Shape::Cursor,
    command: CURSOR_COMMAND,
    hooks: &[
        start("sessionStart"),
        start("beforeSubmitPrompt"),
        edit("postToolUse", "Write"),
        stop("stop"),
    ],
};

/// Cursor applies the rules file to every request.
const CURSOR_RULE: &str = "---\ndescription: How JevGate's findings work and what to do with them\nalwaysApply: true\n---\n\n";
/// The OpenCode plugin, which relays OpenCode's events to `jevgate hook`.
pub(super) const OPENCODE_PLUGIN: &str = include_str!("opencode.js");
const FINDINGS: &str = "how JevGate's findings work";

/// Where the agents keep their files: each agent's directory for this
/// user, and the repository's top level for `--project`.
#[derive(Clone, Debug)]
pub(super) struct Places {
    pub home: PathBuf,
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub gemini: PathBuf,
    pub cursor: PathBuf,
    pub opencode: PathBuf,
    pub root: PathBuf,
}

impl Places {
    /// The agents' defaults under the home directory, moved where an agent
    /// lets an environment variable move them: `CLAUDE_CONFIG_DIR`,
    /// `CODEX_HOME`, and `XDG_CONFIG_HOME` for OpenCode.
    pub fn from_env(root: PathBuf) -> Result<Self> {
        let home = std::env::home_dir()
            .filter(|home| home.is_absolute())
            .context("Cannot find your home directory (set HOME, or USERPROFILE on Windows)")?;
        let moved = |variable: &str| {
            std::env::var_os(variable)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        Ok(Self::under(home, root, moved))
    }

    /// The places under `home`, with `moved` naming a directory an
    /// environment variable moved.
    pub fn under(home: PathBuf, root: PathBuf, moved: impl Fn(&str) -> Option<PathBuf>) -> Self {
        Self {
            claude: moved("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            codex: moved("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            gemini: home.join(".gemini"),
            cursor: home.join(".cursor"),
            opencode: moved("XDG_CONFIG_HOME")
                .unwrap_or_else(|| home.join(".config"))
                .join("opencode"),
            home,
            root,
        }
    }

    /// `path` as a person reads it: under the repository or `~`, else whole.
    pub fn show(&self, path: &Path) -> String {
        if let Ok(rest) = path.strip_prefix(&self.root) {
            return rest.display().to_string();
        }
        match path.strip_prefix(&self.home) {
            Ok(rest) => Path::new("~").join(rest).display().to_string(),
            Err(_) => path.display().to_string(),
        }
    }

    /// Every settings file of an agent that runs `target`'s hooks, whatever
    /// the scope: Cursor also runs Claude Code's.
    pub fn hook_files(&self, target: Target) -> [PathBuf; 2] {
        [false, true].map(|project| paths(target, self, project).0)
    }
}

/// What JevGate writes in one file.
#[derive(Debug)]
pub(super) enum Part {
    /// Its hooks, in a settings file others write too.
    Hooks(&'static HookSet),
    /// Its block, in an instruction file others write too.
    Block,
    /// A file it writes whole, and what the file is for.
    Owned { content: String, what: &'static str },
}

impl Part {
    /// What the part is, for a sentence.
    pub fn describe(&self) -> String {
        match self {
            Self::Hooks(set) => format!("hooks {}", set.events()),
            Self::Block => FINDINGS.into(),
            Self::Owned { what, .. } => (*what).into(),
        }
    }

    /// What `--remove` takes out of a file that stays.
    pub fn noun(&self) -> &'static str {
        match self {
            Self::Hooks(_) => "JevGate's hooks",
            Self::Block => "JevGate's block",
            Self::Owned { .. } => "JevGate's file",
        }
    }
}

/// The files `target` gets, for this user or, with `project`, the repository.
pub(super) fn files(target: Target, places: &Places, project: bool) -> Vec<(PathBuf, Part)> {
    let (settings, instructions) = paths(target, places, project);
    let first = match target {
        Target::Claude => Part::Hooks(&CLAUDE),
        Target::Codex => Part::Hooks(&CODEX),
        Target::Gemini => Part::Hooks(&GEMINI),
        Target::Cursor => Part::Hooks(&CURSOR),
        Target::Opencode => Part::Owned {
            content: OPENCODE_PLUGIN.into(),
            what: "a plugin that runs jevgate hook",
        },
    };
    let mut files = vec![(settings, first)];
    files.extend(instructions.map(|path| (path, instructions_part(target))));
    files
}

/// Where `target`'s hooks (OpenCode: its plugin) and instructions go.
/// Cursor keeps a user's rules in its settings, not in a file.
fn paths(target: Target, places: &Places, project: bool) -> (PathBuf, Option<PathBuf>) {
    let root = &places.root;
    let (hooks, instructions) = match (target, project) {
        (Target::Claude, false) => (
            places.claude.join("settings.json"),
            Some(places.claude.join("rules/jevgate.md")),
        ),
        (Target::Claude, true) => (
            root.join(".claude/settings.json"),
            Some(root.join(".claude/rules/jevgate.md")),
        ),
        (Target::Codex, false) => (
            places.codex.join("hooks.json"),
            Some(places.codex.join("AGENTS.md")),
        ),
        (Target::Codex, true) => (root.join(".codex/hooks.json"), Some(root.join("AGENTS.md"))),
        (Target::Gemini, false) => (
            places.gemini.join("settings.json"),
            Some(places.gemini.join("GEMINI.md")),
        ),
        (Target::Gemini, true) => (
            root.join(".gemini/settings.json"),
            Some(root.join("GEMINI.md")),
        ),
        (Target::Cursor, false) => (places.cursor.join("hooks.json"), None),
        (Target::Cursor, true) => (
            root.join(".cursor/hooks.json"),
            Some(root.join(".cursor/rules/jevgate.mdc")),
        ),
        (Target::Opencode, false) => (
            places.opencode.join("plugins/jevgate.js"),
            Some(places.opencode.join("AGENTS.md")),
        ),
        (Target::Opencode, true) => (
            root.join(".opencode/plugins/jevgate.js"),
            Some(root.join("AGENTS.md")),
        ),
    };
    (hooks, instructions)
}

/// How `target` gets the instructions: a rules file of JevGate's own where
/// the agent reads a directory of them, else a block in its shared file.
fn instructions_part(target: Target) -> Part {
    match target {
        Target::Claude => Part::Owned {
            content: text::INSTRUCTIONS.into(),
            what: FINDINGS,
        },
        Target::Cursor => Part::Owned {
            content: format!("{CURSOR_RULE}{}", text::INSTRUCTIONS),
            what: FINDINGS,
        },
        Target::Codex | Target::Gemini | Target::Opencode => Part::Block,
    }
}
