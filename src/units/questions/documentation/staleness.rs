//! Questions about a section that names paths or scripts the repository lacks.
use super::INSTRUCTIONS;
use serde_json::{Value, json};

/// Whether a section relies on a path or script the repository lacks.
pub fn section_relies() -> Value {
    json!({
        "type": "noul",
        "instructions": {
            "question": "Does `section.text` tell the reader to use something in `missing` as if it exists in this repository now?",
            "note": format!("`missing` lists paths and scripts the section names that this repository does not contain, with what its history shows. `file.title` is the title of the document the section is in. {INSTRUCTIONS}"),
        },
        "criteria": {
            "true": "It tells the reader to open, edit, run or rely on a listed path or script as a current part of this repository, the one the document belongs to.",
            "false": "It names it as part of the reader's own application, such as a file a tutorial, guide or example has the reader create or change, or a command it has the reader run in their project; a file the reader or a command creates; an output; a local or ignored file; an example, placeholder or naming pattern; a name that only looks like a path, such as a module, attribute, route or protocol method; a file of an installed package or another project; or something removed or renamed.",
        },
    })
}

/// What a section treats the names in `missing` as, asked only when the
/// staleness check stays undecided; it can only clear it. Weighing whether
/// a section relies on a name stayed near the middle for protocol methods,
/// example paths and the reader's own commands, while naming what the
/// section treats them as is a choice among distinct kinds.
pub fn missing_role() -> Value {
    let roles: [(&str, &str); 5] = [
        (
            "repository",
            "A current part of this repository that the reader should open, edit, run or rely on.",
        ),
        (
            "reader",
            "Part of the reader's own project, such as a file a guide has them create or a command they run there, or a file a command creates.",
        ),
        ("example", "An example, placeholder or naming pattern."),
        (
            "not_a_file",
            "Not a file of any project: a module, attribute, route, URL path or protocol method, or a file of an installed package or another project.",
        ),
        (
            "removed",
            "Something the section says was removed or renamed.",
        ),
    ];
    json!({
        "type": "choice",
        "instructions": {
            "question": "What does `section.text` treat the names in `missing` as?",
            "note": format!("`missing` lists paths and scripts the section names that this repository does not contain. {INSTRUCTIONS}"),
        },
        "criteria": roles.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
    })
}
