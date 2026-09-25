//! Messages of the documentation rules: instruction sections, large docs, stale and repeated sections, and code comments.
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
    let split = get("split").map(|a| document_split(a, get("kind")));
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
    let conflict = answers.get("conflict").map(|a| disagreement(a));
    let there = format!(
        "section `{}` of `{}`",
        other.symbol.as_deref().unwrap_or(""),
        other.path.display()
    );
    let covers = |q: &str| answers.get(q).map(|a| repeated(a));
    // The note a repetition answer raises, when no repetition is decided.
    let repeated_note = [covers("a_covers"), covers("b_covers")]
        .into_iter()
        .flatten()
        .try_fold(0.0_f64, |most, c| match c {
            Outcome::Review(_) => None,
            Outcome::Note(q) => Some(most.max(q)),
            _ => Some(most),
        });
    let verb = match (conflict, repeated_note) {
        (Some(Outcome::Review(_)), _) => Some("give"),
        // A leaning disagreement words the note it raised.
        (Some(Outcome::Note(q)), Some(most)) if q >= most => Some("may give"),
        _ => None,
    };
    if let Some(verb) = verb {
        return (
            (
                format!(
                    "Section `{name}` and {there} {verb} different values or instructions for the same thing ({p:.2})."
                ),
                "Reconcile the two sections and keep the fact in one place",
            ),
            true,
        );
    }
    let message = match (covers("a_covers"), covers("b_covers")) {
        (Some(Outcome::Review(_)), _) => {
            format!("Section `{name}` states everything {there} states ({p:.2}).")
        }
        (_, Some(Outcome::Review(_))) => {
            format!("{there} states everything section `{name}` states ({p:.2}).")
        }
        (Some(Outcome::Note(_)), _) => {
            format!("Section `{name}` states most or all of what {there} states ({p:.2}).")
        }
        _ => format!("{there} states most or all of what section `{name}` states ({p:.2})."),
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

/// What is wrong with a comment: the question whose outcome raised it, or
/// when the kind of comment decided, that kind (`restates`, `verbose`,
/// `narration`, `history` or `disabled`).
pub(in crate::units) fn comment_reason(answers: &Answers<'_>, documentation: bool) -> &'static str {
    let get = |q: &str| answers.get(q).copied();
    let level = |o: &Outcome| match o {
        Outcome::Review(_) => 3,
        Outcome::Consider(_) => 2,
        Outcome::Note(_) => 1,
        _ => 0,
    };
    let raised = comment_signals(&get, documentation).and_then(|signals| {
        signals
            .into_iter()
            .filter(|(_, o)| level(o) > 0)
            .max_by(|a, b| {
                (level(&a.1), a.1.concern())
                    .partial_cmp(&(level(&b.1), b.1.concern()))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(question, _)| question)
    });
    let kind = || comment_concern_kind(get("kind")).map_or("restates", |(kind, _)| kind);
    match raised.unwrap_or_else(kind) {
        "verbose" => "verbose",
        "narration" => "narration",
        "history" => "history",
        "disabled" => "disabled",
        _ => "restates",
    }
}

/// What is wrong with comments of a reason, said of one comment and of several.
fn reason_words(reason: &str) -> (&'static str, &'static str) {
    match reason {
        "verbose" => (
            "it holds sentences that add nothing",
            "they hold sentences that add nothing",
        ),
        "narration" => (
            "it spells out step by step what the code does",
            "they spell out step by step what the code does",
        ),
        "history" => (
            "it narrates an edit instead of the code as it is",
            "they narrate edits instead of the code as it is",
        ),
        "disabled" => ("it is code turned off", "they are code turned off"),
        _ => ("it repeats the code", "they repeat the code"),
    }
}

/// Lines as a reader names them: `line 4`, `lines 3–8`, `lines 4, 6 and 8`.
fn line_list(locations: &[&crate::schema::Location]) -> String {
    let spans: Vec<String> = locations
        .iter()
        .map(|l| {
            if l.start_line == l.end_line {
                l.start_line.to_string()
            } else {
                format!("{}–{}", l.start_line, l.end_line)
            }
        })
        .collect();
    let plural = spans.len() > 1 || locations.iter().any(|l| l.start_line != l.end_line);
    let joined = match spans.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => spans.join(""),
    };
    format!("{} {joined}", if plural { "lines" } else { "line" })
}

/// The comments of one unit to clean up, grouped by what is wrong with
/// them. The action follows the reasons: a comment that narrates an edit is
/// rewritten to describe the code, since the rest of it often explains the
/// code; a wordy one is shortened; the others are deleted.
pub(in crate::units) fn comment_wording(
    owner: &str,
    listed: &[(&crate::schema::Location, &'static str)],
    strength: Strength,
    p: f64,
) -> Wording {
    let subject = if owner == crate::units::comments::TOP_LEVEL {
        "This file's top-level code".to_string()
    } else {
        format!("`{owner}`")
    };
    let mut reasons: Vec<&'static str> = Vec::new();
    for (_, reason) in listed {
        if !reasons.contains(reason) {
            reasons.push(reason);
        }
    }
    let parts: Vec<String> = reasons
        .iter()
        .map(|reason| {
            let at: Vec<&crate::schema::Location> = listed
                .iter()
                .filter(|(_, r)| r == reason)
                .map(|(l, _)| *l)
                .collect();
            let (one, several) = reason_words(reason);
            let words = if at.len() > 1 { several } else { one };
            format!("at {} {words}", line_list(&at))
        })
        .collect();
    let many = listed.len() > 1;
    let counted = if many {
        format!("{} comments", listed.len())
    } else {
        "a comment".to_string()
    };
    let message = format!(
        "{subject} has {counted} to clean up{}: {}.",
        shown(strength, p),
        parts.join("; ")
    );
    let has = |reason: &str| reasons.contains(&reason);
    let shorten = has("verbose") || has("narration");
    let action = match (strength, many) {
        (Strength::Note, false) => "Optional: delete, shorten or rewrite it",
        (Strength::Note, true) => "Optional: delete, shorten or rewrite them",
        (_, false) if has("history") => {
            "Rewrite the comment to describe the code as it is; version control keeps its history"
        }
        (_, true) if has("history") => {
            "Rewrite the comments that narrate edits to describe the code as it is, and delete or shorten the others"
        }
        (_, false) if shorten => "Shorten the comment to what the code does not already say",
        (_, true) if shorten => {
            "Delete these comments or shorten them to what the code does not already say"
        }
        (_, false) => "Delete the comment",
        (_, true) => "Delete these comments",
    };
    (message, action)
}
