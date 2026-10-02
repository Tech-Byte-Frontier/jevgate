//! Look-here questions: one recall-oriented Noul per unit of the
//! maintainability rules, which flags the unit for a coding agent to verify.
//! The agent fixes what it finds or dismisses the finding with a reason, so a
//! question names the kinds of problems to look for rather than the exact
//! one. On a 32-file sample of a game backend and web client, these flagged
//! the three functions and six of the nine repeats a reviewer wanted changed,
//! where the split Score and statement windows flagged none; with the
//! agent's dismissals, their flags were 27 real, 20 debatable and 11 noise.
//! Examples come from several kinds of programs.
use super::EVIDENCE;
use serde_json::{Value, json};

fn look(question: String, note: &str, yes: (&str, &[&str]), no: (&str, &[&str])) -> Value {
    json!({
        "type": "noul",
        "instructions": {"question": question, "note": format!("{note} {EVIDENCE}").trim()},
        "criteria": {
            "true": {"what": yes.0, "examples": yes.1},
            "false": {"what": no.0, "examples": no.1},
        },
    })
}

/// Whether a function could be made simpler: long, nested, repetitive or
/// mixing jobs. The split Score read a table-driven list of thirty
/// near-identical calls and a long component as one job each.
pub fn function_look(path: &str) -> Value {
    look(
        format!("Could the function in `{path}` be made noticeably simpler to read or change?"),
        "",
        (
            "A maintainer would likely find a simpler form: it is long, deeply nested, repeats the same steps with small changes, or mixes separate jobs.",
            &[
                "A 60-line request handler that validates input, queries the database, formats a report and sends an email",
                "The same five lines written out once per case with one name or value changed",
                "Conditions nested four levels deep before the main work happens",
                "A long list of near-identical calls that a table or a loop could drive",
            ],
        ),
        (
            "It is already about as simple as it can be.",
            &[
                "A short function with one loop, one query or one calculation",
                "A flat sequence of calls to named helpers",
                "Checks before a write, or one transaction, that belong together",
                "One component that only lays out its markup",
            ],
        ),
    )
}

const VALUE_EXAMPLES: [&str; 4] = [
    "A check for one specific customer, region or product id",
    "A server address, account name or absolute path for one machine",
    "An unexplained number such as 86399000 or 0.15 in a calculation",
    "A starting location or default item written as a literal instead of read from data",
];

const FINE_VALUES: [&str; 4] = [
    "Messages, keys, field names, formats and status names",
    "0, 1, -1 and small counts",
    "Values held in named constants or explained by a comment or name",
    "Values in test data",
];

/// Whether a function holds a value worth a look: special-cased,
/// environment-specific, likely to change or unexplained.
pub fn values_look(path: &str) -> Value {
    look(
        format!(
            "Does the function in `{path}` contain a fixed value that a maintainer should look at?"
        ),
        "",
        (
            "A value that singles out one specific record, place or user, is likely to change, would differ between environments, or is an unexplained number that should have a name.",
            &VALUE_EXAMPLES,
        ),
        (
            "Every value explains itself or belongs where it is.",
            &FINE_VALUES,
        ),
    )
}

/// Whether a file's module-level constants fix a value worth a look. A
/// named constant explains a number, so only what changes between
/// environments or singles out one record counts.
pub fn constants_look() -> Value {
    look(
        "Does a constant in `constants` fix a value that a maintainer should look at?".into(),
        "",
        (
            "A constant singles out one specific record, place or user, or holds a value that differs between environments, such as a server address, account or absolute path, fixed in the code.",
            &VALUE_EXAMPLES[..3],
        ),
        (
            "Every constant names a value that is the same wherever the program runs.",
            &[
                "Limits, timeouts and sizes with descriptive names",
                "Routes, file names and formats of the program itself",
                "Addresses read from configuration with a local default",
            ],
        ),
    )
}

/// Whether a file holds parts a maintainer would keep in separate files.
/// `tests` lists a test file's cases rather than an application's members.
pub fn outline_look(tests: bool) -> Value {
    let note = if tests {
        "Members are test cases, with the suite that encloses each one and the functions under test it calls (`subjects`); bodies are not included."
    } else {
        "Members are listed by signature and the names they call, with their length in `lines`; bodies are not included."
    };
    look(
        "Does the file whose members are listed in `members` do several separate kinds of work that a maintainer could keep in separate files?".into(),
        note,
        (
            "It holds two or more separate features, layers, integrations or kinds of work, such as parsing and rendering, or an API client and the reports built on it, even when they are related.",
            &[
                "A payment API client and an invoice PDF renderer in one module",
                "Command-line parsing, configuration loading and report formatting in one file",
                "A test file covering several unrelated modules",
            ],
        ),
        (
            "It holds one feature, type, resource, screen or job and its helpers, even when it is long.",
            &[
                "One class and its methods and helpers",
                "One screen and its parts",
                "One resource's routes and queries",
                "Definitions, settings or messages of one kind",
                "The same kind of code written out for each case, such as one handler per message type",
            ],
        ),
    )
}

/// Whether repeated snippets are one piece of logic kept in several places.
pub fn copies_look() -> Value {
    look(
        "Do the snippets in `sites` repeat the same logic, so that a maintainer should look at keeping it in one place?".into(),
        "`sites` holds each repeated snippet with its path and the function it is in.",
        (
            "They express the same rule, lookup or sequence of steps, so changing one would likely mean changing the others.",
            &[
                "The same eligibility check written in two or more files",
                "Two functions that open, configure, watch and close a connection the same way",
                "The same lookup with a fallback written out at several call sites",
                "Two handlers whose bodies differ only in one name or value",
            ],
        ),
        (
            "The resemblance is incidental, or the code must stay separate.",
            &[
                "Calls to a framework or library API that every caller writes this way",
                "Complementary operations such as encode and decode, or deposit and withdraw",
                "Short statements that look alike but mean different things",
                "Test cases that set up their own data",
            ],
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_look_question_is_one_short_noul_naming_its_evidence() {
        for (body, path) in [
            (function_look("functions[0].source"), "functions[0].source"),
            (values_look("functions[0].source"), "functions[0].source"),
            (constants_look(), "constants"),
            (outline_look(false), "members"),
            (outline_look(true), "members"),
            (copies_look(), "sites"),
        ] {
            assert_eq!(body["type"], "noul");
            super::super::assert_short_question(&body, path);
            for side in ["true", "false"] {
                assert!(!body["criteria"][side]["what"].as_str().unwrap().is_empty());
                assert!(
                    !body["criteria"][side]["examples"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
            }
        }
    }
}
