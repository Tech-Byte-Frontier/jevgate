//! JevGate's hook handlers in an agent's settings file: which handlers are
//! JevGate's, and putting them in or taking them out without touching the
//! rest. A handler is JevGate's when it runs a program named `jevgate` with
//! `hook` as its first argument, wherever that program lives, so what an
//! earlier version or a person wrote by hand is found too.
use super::json::Json;
use anyhow::{Result, bail};

/// How an agent's settings file lists its hooks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    /// Claude Code and Codex: `hooks.EVENT` lists matcher groups, each with a
    /// `hooks` list of handlers.
    Claude,
    /// Gemini CLI: Claude's groups, with named handlers and timeouts in milliseconds.
    Gemini,
    /// Cursor: `hooks.EVENT` lists handlers directly, beside `version: 1`.
    Cursor,
}

/// One hook JevGate installs.
#[derive(Debug)]
pub(super) struct Hook {
    /// The agent's name for the event.
    pub event: &'static str,
    /// The tools the hook runs after, as the agent names them.
    pub matcher: Option<&'static str>,
    /// Seconds the agent lets the hook run: above the hook's own budget.
    pub timeout: u64,
    /// What the agent shows while the hook runs, where it shows one.
    pub status: Option<&'static str>,
}

/// The hooks one agent's settings file gets.
#[derive(Debug)]
pub(super) struct HookSet {
    pub shape: Shape,
    pub command: &'static str,
    pub hooks: &'static [Hook],
}

impl HookSet {
    /// The event names, for a sentence.
    pub fn events(&self) -> String {
        let names: Vec<&str> = self.hooks.iter().map(|hook| hook.event).collect();
        names.join(", ")
    }
}

/// Gemini CLI lists hooks by name: `/hooks disable jevgate` turns off all four.
const GEMINI_NAME: &str = "jevgate";
const GEMINI_DESCRIPTION: &str =
    "JevGate: checks each edit and the end of each turn (jevgate hook)";

/// `settings` with JevGate's handlers replaced by `set`'s: one entry per
/// event, at the end of the event's list. Other handlers keep their place,
/// also in a group they shared with JevGate's, and events JevGate no longer
/// uses lose only its handlers.
pub(super) fn install(settings: &mut Json, set: &HookSet) -> Result<()> {
    if set.shape == Shape::Cursor && settings.get("version").is_none() {
        settings.set("version", 1u64.into());
    }
    if settings.get("hooks").is_none() {
        settings.set("hooks", Json::Object(Vec::new()));
    }
    let Some(Json::Object(events)) = settings.get_mut("hooks") else {
        bail!("its `hooks` is not an object");
    };
    let emptied = strip(events);
    for hook in set.hooks {
        let at = match events.iter().position(|(name, _)| name == hook.event) {
            Some(at) => at,
            None => {
                events.push((hook.event.to_string(), Json::Array(Vec::new())));
                events.len() - 1
            }
        };
        let Some(list) = events[at].1.as_array_mut() else {
            bail!("its `hooks.{}` is not a list", hook.event);
        };
        list.push(entry(set, hook));
    }
    events.retain(|(name, list)| !(emptied.contains(name) && list.is_empty()));
    Ok(())
}

/// `settings` without JevGate's handlers, dropping the groups, event lists
/// and `hooks` object that only held them. The number removed.
pub(super) fn remove(settings: &mut Json) -> usize {
    let before = handlers(settings);
    let Some(Json::Object(events)) = settings.get_mut("hooks") else {
        return 0;
    };
    let emptied = strip(events);
    events.retain(|(name, list)| !(emptied.contains(name) && list.is_empty()));
    let empty = events.is_empty();
    let removed = before - handlers(settings);
    if removed > 0 && empty {
        settings.remove("hooks");
    }
    removed
}

/// How many of `settings`' hook handlers are JevGate's, in a group's
/// `hooks` or, where handlers are listed directly, as the entry itself.
pub(super) fn handlers(settings: &Json) -> usize {
    let Some(Json::Object(events)) = settings.get("hooks") else {
        return 0;
    };
    events
        .iter()
        .filter_map(|(_, list)| match list {
            Json::Array(entries) => Some(entries),
            _ => None,
        })
        .flatten()
        .map(|entry| match entry.get("hooks") {
            Some(Json::Array(handlers)) => handlers.iter().filter(|h| is_jevgate(h)).count(),
            _ => usize::from(is_jevgate(entry)),
        })
        .sum()
}

/// Whether the document holds nothing once JevGate's hooks are gone: no
/// keys, or only Cursor's `version`.
pub(super) fn bare(settings: &Json) -> bool {
    match settings {
        Json::Object(entries) => entries.iter().all(|(key, _)| key == "version"),
        _ => false,
    }
}

/// Take JevGate's handlers out of every event list: from a group's `hooks`
/// (dropping a group left empty) or, where handlers are listed directly,
/// the entry itself. The events whose lists changed.
fn strip(events: &mut [(String, Json)]) -> Vec<String> {
    let mut changed = Vec::new();
    for (name, list) in events.iter_mut() {
        let Some(entries) = list.as_array_mut() else {
            continue;
        };
        let before = entries.clone();
        entries.retain_mut(|entry| match entry.get_mut("hooks") {
            Some(Json::Array(handlers)) => {
                let had = handlers.len();
                handlers.retain(|handler| !is_jevgate(handler));
                had == handlers.len() || !handlers.is_empty()
            }
            _ => !is_jevgate(entry),
        });
        if *entries != before {
            changed.push(name.clone());
        }
    }
    changed
}

/// The entry `set` adds for `hook`, in its agent's shape.
fn entry(set: &HookSet, hook: &Hook) -> Json {
    let handler = match set.shape {
        Shape::Claude => Json::object(
            [
                ("type", "command".into()),
                ("command", set.command.into()),
                ("timeout", hook.timeout.into()),
            ]
            .into_iter()
            .chain(hook.status.map(|status| ("statusMessage", status.into()))),
        ),
        Shape::Gemini => Json::object([
            ("name", GEMINI_NAME.into()),
            ("type", "command".into()),
            ("command", set.command.into()),
            ("timeout", (hook.timeout * 1000).into()),
            ("description", GEMINI_DESCRIPTION.into()),
        ]),
        Shape::Cursor => {
            return Json::object(
                [("command", set.command.into())]
                    .into_iter()
                    .chain(hook.matcher.map(|matcher| ("matcher", matcher.into())))
                    .chain([("timeout", hook.timeout.into())]),
            );
        }
    };
    Json::object(
        hook.matcher
            .map(|matcher| ("matcher", matcher.into()))
            .into_iter()
            .chain([("hooks", Json::Array(vec![handler]))]),
    )
}

/// Whether `handler` runs `jevgate hook`: in exec form (`command` plus
/// `args`) or as a shell command whose first word is the program.
pub(super) fn is_jevgate(handler: &Json) -> bool {
    let Some(command) = handler.get("command").and_then(Json::as_str) else {
        return false;
    };
    match handler.get("args") {
        Some(Json::Array(args)) => {
            is_jevgate_program(command) && args.first().and_then(Json::as_str) == Some("hook")
        }
        _ => {
            let (program, rest) = first_word(command);
            is_jevgate_program(program) && first_word(rest).0 == "hook"
        }
    }
}

/// The first word of a shell command, quoted or not, and what follows it.
fn first_word(command: &str) -> (&str, &str) {
    let command = command.trim_start();
    match command.chars().next() {
        Some(quote @ ('"' | '\'')) => {
            let inner = &command[1..];
            match inner.find(quote) {
                Some(end) => (&inner[..end], &inner[end + 1..]),
                None => (inner, ""),
            }
        }
        _ => {
            let end = command.find(char::is_whitespace).unwrap_or(command.len());
            (&command[..end], &command[end..])
        }
    }
}

/// Whether `program` names `jevgate` or `jevgate.exe`, in any directory.
fn is_jevgate_program(program: &str) -> bool {
    let name = program.rsplit(['/', '\\']).next().unwrap_or(program);
    name == "jevgate" || name.eq_ignore_ascii_case("jevgate.exe")
}
