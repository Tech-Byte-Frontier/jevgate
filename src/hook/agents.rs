//! Each agent's hook protocol: which agent sent an event, what the event
//! means, and how a reply is written for it. Claude Code's shape is the
//! common one: Codex and Gemini CLI read the same reply fields (echoing their
//! own event names), and so does JevGate's OpenCode plugin. Cursor's own
//! hooks.json uses different fields, and Copilot CLI and VS Code, which run
//! Claude-format hooks, read context and blocks where Claude does not.
use serde_json::{Value, json};
use std::path::PathBuf;

/// An agent whose hook protocol `jevgate hook` speaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Agent {
    /// Claude Code, and Devin CLI, which loads its hooks
    Claude,
    /// OpenAI Codex
    Codex,
    /// Gemini CLI
    Gemini,
    /// Cursor, configured in its own hooks.json
    Cursor,
    /// OpenCode, through JevGate's plugin
    Opencode,
    /// GitHub Copilot CLI and VS Code, running Claude Code's hooks
    Copilot,
}

/// What an event asks of JevGate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    SessionStart,
    TurnStart,
    AfterEdit,
    Stop,
    /// An event JevGate does not act on; the reply is empty.
    Other,
}

/// One hook event, whatever the agent.
#[derive(Clone, Debug)]
pub(super) struct Event {
    pub agent: Agent,
    /// The agent's name for the event, repeated in the reply.
    pub name: String,
    pub kind: Kind,
    pub session: String,
    /// The session's working directory, when the event names one.
    pub cwd: Option<PathBuf>,
    /// A turn start's prompt.
    pub prompt: String,
    /// The files an edit wrote, as the agent named them.
    pub files: Vec<PathBuf>,
    /// At a stop: the agent is already continuing because a stop hook blocked.
    pub continued: bool,
    /// At a stop: the turn completed (Cursor also stops on an abort or an error).
    pub completed: bool,
}

/// What the hook tells the agent and the person, in any agent's format.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Reply {
    /// At a stop: keep the agent working, with `agent` as the reason.
    pub block: bool,
    /// Text for the agent: findings, or a notice.
    pub agent: Option<String>,
    /// Text for the person: why nothing was checked, what did not block.
    pub user: Option<String>,
}

/// Every agent's names for the events JevGate acts on. Hosts load each
/// other's hook files (Cursor runs `~/.claude/settings.json`, Copilot CLI the
/// repository's `.claude/settings.json`), so a name is read whoever sent it.
const EVENTS: [(Kind, &[&str]); 4] = [
    (
        Kind::SessionStart,
        &["SessionStart", "sessionStart", "session.created"],
    ),
    (
        Kind::TurnStart,
        &[
            "UserPromptSubmit",
            "BeforeAgent",
            "beforeSubmitPrompt",
            "chat.message",
        ],
    ),
    (
        Kind::AfterEdit,
        &[
            "PostToolUse",
            "AfterTool",
            "postToolUse",
            "tool.execute.after",
        ],
    ),
    (Kind::Stop, &["Stop", "AfterAgent", "stop", "session.idle"]),
];

/// Tools that write files: Claude Code's (which Copilot reports too),
/// Codex's `apply_patch`, Gemini CLI's, Cursor's `Write` and OpenCode's.
const EDIT_TOOLS: [&str; 13] = [
    "Edit",
    "Write",
    "MultiEdit",
    "NotebookEdit",
    "apply_patch",
    "write_file",
    "replace",
    "edit",
    "write",
    "patch",
    "multiedit",
    "create",
    "str_replace_editor",
];

/// Event names only Gemini CLI uses.
const GEMINI_EVENTS: [&str; 7] = [
    "BeforeAgent",
    "AfterAgent",
    "BeforeTool",
    "AfterTool",
    "BeforeModel",
    "AfterModel",
    "BeforeToolSelection",
];

/// The agent that sent `input`, from the fields and event names only it
/// uses. Copilot CLI and VS Code send Claude's event names with a
/// `timestamp` and no `permission_mode`, which Claude Code sends at a
/// prompt, after a tool and at a stop; Gemini CLI sends a timestamp too, but
/// under its own names except at a session's start, where its reply reads
/// the same.
pub(super) fn detect(input: &Value) -> Agent {
    let name = input["hook_event_name"].as_str().unwrap_or_default();
    if input.get("conversation_id").is_some() || input.get("cursor_version").is_some() {
        Agent::Cursor
    } else if input.get("sessionID").is_some() || name.contains('.') {
        Agent::Opencode
    } else if input.get("turn_id").is_some() {
        Agent::Codex
    } else if GEMINI_EVENTS.contains(&name) {
        Agent::Gemini
    } else if input.get("timestamp").is_some() && input.get("permission_mode").is_none() {
        Agent::Copilot
    } else {
        Agent::Claude
    }
}

/// What `name` asks of JevGate, given the tool an after-tool event ran.
fn kind(name: &str, tool: &str) -> Kind {
    match EVENTS.iter().find(|(_, names)| names.contains(&name)) {
        Some((Kind::AfterEdit, _)) if !EDIT_TOOLS.contains(&tool) => Kind::Other,
        Some((kind, _)) => *kind,
        None => Kind::Other,
    }
}

pub(super) fn event(agent: Agent, input: &Value) -> Event {
    let name = input["hook_event_name"].as_str().unwrap_or_default();
    let (tool, arguments) = match agent {
        Agent::Opencode => (&input["tool"], &input["args"]),
        _ => (&input["tool_name"], &input["tool_input"]),
    };
    let text = |key: &str| input[key].as_str().map(str::to_string);
    let first = |keys: &[&str]| keys.iter().find_map(|key| text(key)).unwrap_or_default();
    Event {
        agent,
        name: name.to_string(),
        kind: kind(name, tool.as_str().unwrap_or_default()),
        session: first(&["session_id", "conversation_id", "sessionID"]),
        cwd: first_directory(input).map(PathBuf::from),
        prompt: first(&["prompt"]),
        files: edited_files(arguments),
        continued: input["stop_hook_active"].as_bool() == Some(true)
            || input["continued"].as_bool() == Some(true)
            || input["loop_count"].as_u64().is_some_and(|n| n > 0),
        completed: input["status"]
            .as_str()
            .is_none_or(|status| status == "completed"),
    }
}

/// The session's directory: `cwd`, OpenCode's `directory`, or Cursor's first
/// workspace root.
fn first_directory(input: &Value) -> Option<&str> {
    input["cwd"]
        .as_str()
        .or_else(|| input["directory"].as_str())
        .or_else(|| input["workspace_roots"][0].as_str())
        .filter(|dir| !dir.is_empty())
}

/// The files a tool call wrote: a path argument (each agent names it its own
/// way), or the files a patch adds, updates or moves to.
fn edited_files(arguments: &Value) -> Vec<PathBuf> {
    const PATHS: [&str; 5] = [
        "file_path",
        "notebook_path",
        "filePath",
        "path",
        "target_file",
    ];
    const PATCHES: [&str; 2] = ["command", "patchText"];
    if let Some(path) = PATHS.iter().find_map(|key| arguments[key].as_str()) {
        return vec![PathBuf::from(path)];
    }
    PATCHES
        .iter()
        .filter_map(|key| arguments[key].as_str())
        .flat_map(patched_files)
        .collect()
}

/// Paths in an `apply_patch` envelope (`*** Update File: src/a.rs`): those
/// added, updated or moved to. Headers start their line; a hunk's lines
/// start with a space, `+` or `-`, so a file quoting a header is not read as
/// one. A moved file's old path no longer exists and is dropped with the
/// other paths outside the repository.
fn patched_files(patch: &str) -> Vec<PathBuf> {
    const HEADERS: [&str; 3] = ["*** Add File: ", "*** Update File: ", "*** Move to: "];
    patch
        .lines()
        .filter_map(|line| HEADERS.iter().find_map(|header| line.strip_prefix(header)))
        .map(|path| PathBuf::from(path.trim()))
        .collect()
}

/// Whether `agent` reads context for the agent at events of `kind` without
/// continuing a stopped turn. Cursor's prompt hook takes none.
pub(super) fn has_context(agent: Agent, kind: Kind) -> bool {
    match kind {
        Kind::SessionStart | Kind::AfterEdit => true,
        Kind::TurnStart => agent != Agent::Cursor,
        Kind::Stop | Kind::Other => false,
    }
}

/// The JSON reply to `event`. Only a stop blocks, through the reply's
/// decision; context is added only where it does not continue a stopped turn.
pub(super) fn render(event: &Event, reply: &Reply) -> Value {
    if event.agent == Agent::Cursor {
        return cursor(event, reply);
    }
    let mut out = json!({});
    let context = reply
        .agent
        .as_ref()
        .filter(|_| !reply.block && has_context(event.agent, event.kind));
    if reply.block {
        out["decision"] = json!("block");
        out["reason"] = json!(reply.agent);
    } else if let Some(text) = context {
        out["hookSpecificOutput"] = json!({"hookEventName": event.name, "additionalContext": text});
    }
    if event.agent == Agent::Copilot {
        copilot(&mut out, event, reply, context);
    }
    if let Some(text) = &reply.user {
        out["systemMessage"] = json!(text);
    }
    out
}

/// Where Copilot CLI and VS Code read what Claude reads elsewhere: Copilot
/// CLI takes context at the top level, and VS Code a block inside
/// `hookSpecificOutput`. Neither Claude Code nor Codex, whose parser rejects
/// unknown fields, is ever sent these.
fn copilot(out: &mut Value, event: &Event, reply: &Reply, context: Option<&String>) {
    if reply.block {
        out["hookSpecificOutput"] =
            json!({"hookEventName": event.name, "decision": "block", "reason": reply.agent});
    } else if let Some(text) = context {
        out["additionalContext"] = json!(text);
    }
}

/// Cursor's fields: `continue` for a prompt, `additional_context` after a
/// tool or at session start, and `followup_message` to keep a stopped agent
/// working. It shows people nothing from these hooks; `jevgate hook` writes
/// the person's message to stderr, which its Hooks output channel shows.
fn cursor(event: &Event, reply: &Reply) -> Value {
    match event.kind {
        Kind::TurnStart => json!({"continue": true}),
        Kind::Stop if reply.block => json!({"followup_message": reply.agent}),
        Kind::SessionStart | Kind::AfterEdit => reply
            .agent
            .as_ref()
            .map_or_else(|| json!({}), |text| json!({"additional_context": text})),
        Kind::Stop | Kind::Other => json!({}),
    }
}
