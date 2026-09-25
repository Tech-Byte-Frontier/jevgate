//! Questions about the heading sections of agent instruction files.
use super::INSTRUCTIONS;
use crate::units::questions::choose_id;
use serde_json::{Value, json};

fn section_noul(question: String, yes: &str, no: &str) -> Value {
    json!({
        "type": "noul",
        "instructions": {"question": question, "note": INSTRUCTIONS},
        "criteria": {"true": yes, "false": no},
    })
}

/// Whether an agent could learn a section of its instructions from the
/// repository's own files. The top level names what those files show.
pub fn instructions_inferable(section: &str) -> Value {
    json!({
        "type": "score",
        "instructions": {
            "question": format!("Could an agent working in this repository learn what `{section}.text` says by reading the files `project` lists?"),
            "note": format!("`project` lists the repository's manifests, with their dependencies and scripts, and its directories. {INSTRUCTIONS}"),
        },
        "criteria": [
            "No. It states commands, conventions, constraints, decisions or workflows specific to this project that its files do not show, such as why something is done, steps required before a commit, or things to avoid.",
            "Partly. Most of it is specific to this project, but some sentences restate what the files show.",
            "Yes. It restates what the files show, such as the language, framework, dependencies, directory layout, what each file contains, or the standard commands of the build tool.",
        ],
    })
}

/// Description without instructions: what an agent reads from the files
/// itself. Facts from outside the repository are not descriptions of it.
pub fn instructions_describes(section: &str) -> Value {
    section_noul(
        format!(
            "Does `{section}.text` only describe the project, such as its stack, dependencies, layout or what its files contain, without telling the reader how to work?"
        ),
        "It only describes what the project is made of or where things are: languages, frameworks, dependencies, packages, directories, what files or modules contain, or an overview of how parts fit together.",
        "It tells the reader what to do, avoid or run, or states a decision, constraint, convention or format to follow, even alongside description. Facts about people, services or data outside the repository are not descriptions of its files.",
    )
}

/// Asked only when the repository has manifests.
pub fn instructions_commands(section: &str) -> Value {
    section_noul(
        format!(
            "Is `{section}.text` a list of commands that `project.manifests` already shows as scripts, targets or the standard commands of its tools?"
        ),
        "It lists commands, and each one is a script or target in `project.manifests` or a standard command of the build tool those manifests use, with no more than what each command runs.",
        "It is prose, rules or explanations rather than a list of commands, or it names a command, flag, order or condition that `project.manifests` does not show, such as which check must pass before a commit.",
    )
}

pub fn instructions_generic(section: &str) -> Value {
    section_noul(
        format!("Does `{section}.text` give only advice that would apply to any software project?"),
        "Only general advice, such as write clean code, add tests, handle errors or follow best practices, with nothing that names this project's files, commands, tools or decisions.",
        "It names or depends on something specific to this project: a file, command, tool, service, convention or decision, even alongside general advice.",
    )
}

pub fn instructions_history(section: &str) -> Value {
    section_noul(
        format!(
            "Does `{section}.text` record past work instead of instructions, such as a log, results or a status update?"
        ),
        "It reports what was done, tried, measured or decided at some point: dated entries, change logs, completed task lists, experiment narratives, benchmark results or current progress.",
        "It tells the reader how to work: instructions, constraints, commands or explanations that stay true until the project changes. A short reason for a rule is not a record of past work.",
    )
}

/// What a section mainly is, asked of a section whose signals stay
/// undecided: on vercel/ai, sections such as an API table or an import map
/// stayed near the middle on "only describes" and "restates", while naming
/// what the section does is a choice among distinct kinds.
pub fn instructions_kind(section: &str) -> Value {
    let kinds: [(&str, &str); 5] = [
        (
            "instructions",
            "Tells the reader how to work in this project: conventions, constraints, decisions, required steps, things to avoid, or when a command must run.",
        ),
        (
            "description",
            "Describes what the project is made of or where things are: its stack, dependencies, packages, directories, APIs or what its files contain, without telling the reader what to do.",
        ),
        (
            "commands",
            "Lists commands with what each one runs, without saying when one must run.",
        ),
        (
            "generic",
            "Gives advice that applies to any software project, such as write tests or keep changes small.",
        ),
        (
            "record",
            "Records past work: a log, results or a status update.",
        ),
    ];
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("Which best describes what `{section}.text` mainly does?"),
            "note": INSTRUCTIONS,
        },
        "criteria": kinds.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
    })
}

/// Asked only when the repository configures formatting or lint tools.
pub fn instructions_enforced(section: &str) -> Value {
    section_noul(
        format!(
            "Is `{section}.text` only formatting or style rules that a tool in `linters` already checks or fixes?"
        ),
        "Every rule it states is formatting or style that the listed tools check or fix with their usual settings, such as whitespace, indentation, quotes, semicolons, import order or unused code.",
        "At least one of its statements is about something other than formatting and style, such as behavior, APIs, error handling, architecture, workflow or commands; or it asks for a convention such tools check only when configured for it, such as how files are named; or it only says which tool to run and when.",
    )
}

/// Asked only for text loaded at the start of every session: whether it
/// belongs in instructions that load only for one directory.
pub fn instructions_scope(section: &str, directories: &[String]) -> Value {
    choose_id(
        &format!(
            "Does all of `{section}.text` apply only to work in one directory of `project.directories`, and which one?"
        ),
        format!(
            "This file loads at the start of every session. Options are the entries in `project.directories`. {INSTRUCTIONS}"
        ),
        directories,
        "It applies to work anywhere in the repository, or to more than one directory.",
    )
}
