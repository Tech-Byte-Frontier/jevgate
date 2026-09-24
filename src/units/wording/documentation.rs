//! Messages of the documentation rules: instruction sections, large docs, stale and repeated sections.
use super::*;

/// One instruction-section question and its words.
struct SectionSignal {
    question: &'static str,
    finding: &'static str,
    note: &'static str,
    action: &'static str,
}

const SECTION_SIGNALS: [SectionSignal; 7] = [
    SectionSignal {
        question: "inferable",
        finding: "restates what the repository's files show",
        note: "partly restates what the repository's files show",
        action: "Remove what agents learn from the code; keep project-specific instructions",
    },
    SectionSignal {
        question: "describes",
        finding: "only describes the project, which agents read from its files",
        note: "only describes the project, which agents read from its files",
        action: "Remove the description; keep instructions agents cannot infer",
    },
    SectionSignal {
        question: "commands",
        finding: "only lists commands the manifests already show",
        note: "only lists commands the manifests already show",
        action: "Remove the command list, or keep only when each command must run",
    },
    SectionSignal {
        question: "generic",
        finding: "gives only advice that applies to any project",
        note: "gives only advice that applies to any project",
        action: "Remove the generic advice",
    },
    SectionSignal {
        question: "history",
        finding: "records past work instead of instructions",
        note: "records past work instead of instructions",
        action: "Move the record out of the instructions, into notes or commit history",
    },
    SectionSignal {
        question: "enforced",
        finding: "asks for rules the configured linters already check",
        note: "asks for rules the configured linters already check",
        action: "Remove the rules the linters enforce",
    },
    SectionSignal {
        question: "scope",
        finding: "applies to one directory but loads in every session",
        note: "applies to one directory but loads in every session",
        action: "Optional: move it to instructions that load only for that directory",
    },
];

/// Each instruction-section signal that reached this strength, with the
/// harnesses that load the section and its estimated size.
pub(in crate::units) fn section_wording(
    name: &str,
    detail: &Detail,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> Wording {
    let get = |q: &str| answers.get(q).copied();
    let reached: Vec<&SectionSignal> = section_signals(&get)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, outcome)| {
            matches!(
                (strength, outcome),
                (Strength::Consider, Outcome::Consider(_)) | (Strength::Note, Outcome::Note(_))
            )
        })
        .filter_map(|(question, _)| SECTION_SIGNALS.iter().find(|s| s.question == question))
        .collect();
    let directory = crate::units::outcome::choice(get("scope")).map(|(dir, _)| dir);
    let reasons: Vec<String> = reached
        .iter()
        .map(|signal| match (signal.question, directory) {
            ("scope", Some(dir)) => {
                format!("applies only to work in `{dir}` but loads in every session")
            }
            _ if strength == Strength::Note => signal.note.to_string(),
            _ => signal.finding.to_string(),
        })
        .collect();
    // A block of a long section is named "heading, line N".
    let subject = match name.split_once(", line ") {
        Some((heading, line)) if heading == crate::units::instructions::PREAMBLE => {
            format!("The text at line {line}, before the first heading,")
        }
        Some((heading, line)) => format!("Section `{heading}` at line {line}"),
        None if name == crate::units::instructions::PREAMBLE => format!("The {name}"),
        None => format!("Section `{name}`"),
    };
    let load = match detail {
        Detail::Section { tokens, loaded } if !loaded.is_empty() => {
            format!(" {loaded} (about {tokens} tokens).")
        }
        _ => String::new(),
    };
    (
        format!(
            "{subject} {}{}.{load}",
            reasons.join("; "),
            shown(strength, p)
        ),
        match (strength, reached.first()) {
            (Strength::Note, Some(signal)) if signal.question == "scope" => signal.action,
            (Strength::Note, _) => "Optional: trim what agents learn from the code",
            (_, Some(signal)) => signal.action,
            (_, None) => "Review the section",
        },
    )
}

/// A large document to split, naming the part the locate follow-up chose,
/// or one that mainly records past work.
pub(in crate::units) fn document_wording(
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
    part: Option<&Block>,
) -> Wording {
    let get = |q: &str| answers.get(q).copied();
    let split = get("split").map(benefit);
    let reached = |outcome: Option<Outcome>| {
        matches!(
            (strength, outcome),
            (Strength::Consider, Some(Outcome::Consider(_)))
                | (Strength::Note, Some(Outcome::Note(_)))
        )
    };
    if reached(split.map(|o| match o {
        Outcome::Review(p) => Outcome::Consider(p),
        other => other,
    })) {
        let part = part
            .and_then(|b| b.location.symbol.as_deref())
            .map_or(String::new(), |h| {
                format!("; `{h}` would be most useful as its own document")
            });
        return if strength == Strength::Note {
            (
                format!("`{name}` has a section that could live elsewhere{part}."),
                "Optional: move that section to its own document",
            )
        } else {
            (
                format!("`{name}` holds several unrelated subjects ({p:.2}){part}."),
                "Split the document by subject and link the parts",
            )
        };
    }
    let likely = if strength == Strength::Note {
        " may"
    } else {
        ""
    };
    (
        format!(
            "`{name}`{likely} mainly records past work, such as dated plans, completed tasks or logs{}.",
            shown(strength, p)
        ),
        "Remove finished plans and logs, or move them out of the living documentation",
    )
}

/// A plan whose work Git shows finished, with the facts that show it.
pub(in crate::units) fn plan_wording(name: &str, facts: &[String], p: f64) -> Wording {
    (
        format!(
            "`{name}` is a plan whose work is finished: {} ({p:.2}).",
            facts.join("; ")
        ),
        "Delete the finished plan, or move it out of the living documentation",
    )
}

/// A section that tells the reader to use a path or script that is gone.
pub(in crate::units) fn stale_wording(name: &str, missing: &[String], p: f64) -> Wording {
    (
        format!(
            "Section `{name}` tells the reader to use {} ({p:.2}).",
            missing.join(", ")
        ),
        "Update the section to the current path or command, or remove it",
    )
}

/// A section repeated by, or disagreeing with, a section of another document;
/// true when the pair disagrees.
pub(in crate::units) fn doc_pair_wording(
    name: &str,
    other: &crate::schema::Location,
    answers: &Answers<'_>,
    p: f64,
) -> (Wording, bool) {
    let decided = |q: &str| {
        answers
            .get(q)
            .is_some_and(|a| matches!(noul(a), Outcome::Review(_)))
    };
    let there = format!(
        "section `{}` of `{}`",
        other.symbol.as_deref().unwrap_or(""),
        other.path.display()
    );
    if decided("conflict") {
        return (
            (
                format!(
                    "Section `{name}` and {there} give different values or instructions for the same thing ({p:.2})."
                ),
                "Reconcile the two sections and keep the fact in one place",
            ),
            true,
        );
    }
    let message = if decided("a_covers") {
        format!("Section `{name}` states everything {there} states ({p:.2}).")
    } else {
        format!("{there} states everything section `{name}` states ({p:.2}).")
    };
    (
        (
            capitalized(&message),
            "Keep one copy and link to it from the other document",
        ),
        false,
    )
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or(String::new(), |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}
