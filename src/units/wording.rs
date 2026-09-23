//! The message and recommended action of each kind of finding.
use super::{
    Block, Detail, GroupInfo,
    outcome::{
        Answers, Outcome, benefit, levels, noul, origin_outcome, section_signals, value_signals,
    },
};
use crate::catalog;
use crate::schema::{Answer, Strength};

/// A finding's message and the action it recommends.
pub(super) type Wording = (String, &'static str);

/// Splitting, or flattening when only the flatten Score reached this strength,
/// naming the located block when there is one.
pub(super) fn function_wording(
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
    block: Option<&Block>,
) -> Wording {
    let reached = |question: &str| {
        answers
            .get(question)
            .map(|a| benefit(a))
            .is_some_and(|outcome| match strength {
                Strength::Review => matches!(outcome, Outcome::Review(_)),
                Strength::Consider => matches!(outcome, Outcome::Consider(_)),
                Strength::Note => matches!(outcome, Outcome::Note(_)),
            })
    };
    let flattening = reached("flatten") && !reached("split");
    let located = block.map_or(String::new(), |block| {
        let l = &block.location;
        format!(
            " Lines {}–{} would be most useful as their own function.",
            l.start_line, l.end_line
        )
    });
    match (strength, flattening) {
        (Strength::Review, false) => (
            format!(
                "`{name}` mixes separate jobs in long blocks; splitting it would make it easier to understand ({p:.2}).{located}"
            ),
            if block.is_some() {
                "Extract the located block into a named function"
            } else {
                "Extract each separate job into its own named function"
            },
        ),
        (Strength::Review, true) => (
            format!("`{name}` has nested or repeated branches that hide its main path ({p:.2})."),
            "Flatten the control flow with guard clauses, early returns or a lookup table",
        ),
        (Strength::Consider, false) => {
            let top = answers
                .get("split")
                .and_then(|a| levels(a))
                .map_or(p, |[_, _, top]| top);
            (
                format!(
                    "`{name}` likely mixes separate jobs ({top:.2}); splitting it may make it easier to understand.{located}"
                ),
                if block.is_some() {
                    "Consider extracting the located block into a named function"
                } else {
                    "Consider extracting each separate job into its own named function"
                },
            )
        }
        (Strength::Consider, true) => (
            format!("`{name}` has branching that likely hides its main path ({p:.2})."),
            "Consider guard clauses, early returns or a lookup table",
        ),
        (Strength::Note, false) => (
            format!("`{name}` reads well as it is; one block could be named as a helper ({p:.2})."),
            "Optional: extract that block if it grows",
        ),
        (Strength::Note, true) => (
            format!("`{name}` is easy to follow; one condition could return early ({p:.2})."),
            "Optional: a guard clause or early return",
        ),
    }
}

/// The file-wide concern, naming the group chosen as the module when there is
/// one. A note says why: the file is coherent as it is, or the proposed group
/// has no users of its own elsewhere.
pub(super) fn outline_wording(
    chosen: Option<&GroupInfo>,
    strength: Strength,
    own_users: bool,
    p: f64,
) -> Wording {
    let detail = chosen.map_or(String::new(), |group| {
        let shown: Vec<_> = group
            .names
            .iter()
            .take(6)
            .map(|n| format!("`{n}`"))
            .collect();
        let more = group.names.len().saturating_sub(shown.len());
        let more = if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        };
        format!(
            " {} ({}{more}) would be most useful as its own module.",
            group.id,
            shown.join(", ")
        )
    });
    match strength {
        Strength::Note if !own_users => (
            format!(
                "Some members of this file could live in a separate module ({p:.2}), but no other file uses them apart from the rest, so a module would gain little.{detail}"
            ),
            "Optional: keep the file whole until another file needs that group alone",
        ),
        Strength::Note => (
            format!(
                "This file is coherent as it is; a small set of members could live elsewhere ({p:.2}).{detail}"
            ),
            "Optional: move that set of members if it grows",
        ),
        Strength::Consider => (
            format!("Some members of this file could live in a separate module ({p:.2}).{detail}"),
            "Consider moving that set of members into its own module",
        ),
        Strength::Review => (
            format!("This file holds two or more unrelated responsibilities ({p:.2}).{detail}"),
            if chosen.is_some() {
                "Move that group into its own module"
            } else {
                "Split the file along its separate purposes"
            },
        ),
    }
}

/// `sites` is (within one test, owner in test code, every site in a test case).
pub(super) fn pair_wording(
    name: &str,
    differences: &[crate::analysis::clones::Difference],
    sites: (bool, bool, bool),
    strength: Strength,
    p: f64,
) -> Wording {
    let (within_test, in_tests, in_cases) = sites;
    let renamed = if differences.is_empty() {
        String::new()
    } else {
        let shown: Vec<_> = differences
            .iter()
            .take(6)
            .map(|d| format!("`{}`→`{}`", d.a, d.b))
            .collect();
        format!(" Differences: {}.", shown.join(", "))
    };
    let place = if within_test {
        "inside one test"
    } else {
        "across test cases"
    };
    match strength {
        Strength::Note => (
            format!(
                "{name} repeat steps {place}; writing each case out is common in tests ({p:.2}).{renamed}"
            ),
            "Optional: a fixture, helper or table of cases if the steps grow",
        ),
        Strength::Consider if in_cases => (
            format!("{name} repeat the same steps {place} ({p:.2}).{renamed}"),
            if within_test {
                "Consider a table of cases or a local helper for the repeated steps"
            } else {
                "Consider a fixture or helper for the repeated steps"
            },
        ),
        Strength::Review => (
            format!("{name} perform the same steps for the same purpose ({p:.2}).{renamed}"),
            if in_tests {
                "Share the steps through a fixture, helper or parameterized test"
            } else {
                "Move the shared steps into one implementation"
            },
        ),
        Strength::Consider => (
            format!(
                "{name} repeat related steps; a person should decide whether they belong together ({p:.2}).{renamed}"
            ),
            "Decide whether one implementation should serve both",
        ),
    }
}

/// A question in plain words, for listing what a unit left undecided.
pub(super) fn question_label(question: &str) -> &str {
    match question {
        "split" => "splitting",
        "flatten" => "flattening",
        "same" => "same steps",
        "environment" => "environment value",
        "magic" => "unnamed value",
        "special" => "special case",
        "interpreted" => "variable in interpreted text",
        "resource" => "variable in a path or URL",
        "origin" => "origin of values",
        "handled" => "values bound or checked",
        "logs_secret" => "secret in logs",
        "error_details" => "error details to clients",
        "weakened" => "weak setting",
        "own_logic" => "recomputed expected value",
        "mock_only" => "checks only its mocks",
        "overlap" => "overlapping tests",
        "inferable" => "restates the repository",
        "describes" => "description only",
        "commands" => "commands the manifests show",
        "generic" => "generic advice",
        "history" => "past work",
        "enforced" => "rule linters check",
        other => other,
    }
}

/// One hardcoded-value question and its words.
struct ValueSignal {
    question: &'static str,
    finding: &'static str,
    note: &'static str,
    action: &'static str,
}

const VALUE_SIGNALS: [ValueSignal; 3] = [
    ValueSignal {
        question: "environment",
        finding: "fixes a value that differs between deployments",
        note: "has a local default that configuration could own",
        action: "Read the value from configuration or the environment",
    },
    ValueSignal {
        question: "magic",
        finding: "uses a value whose meaning a reader must guess",
        note: "has a value that could be named, though its context explains it",
        action: "Give the value a descriptive constant name",
    },
    ValueSignal {
        question: "special",
        finding: "special-cases one specific identity",
        note: "special-cases one specific identity",
        action: "Move the special case into data or configuration",
    },
];

/// Each hardcoded-value signal that reached this strength, in plain words; the
/// first one's remedy is the action. A note from an undecided answer that
/// leans toward the concern says the answer was split.
pub(super) fn values_wording(
    name: &str,
    detail: &Detail,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> Wording {
    let get = |q: &str| answers.get(q).copied();
    let signals = value_signals(&get, detail, true).unwrap_or_default();
    let reached: Vec<(&ValueSignal, bool)> = signals
        .iter()
        .filter(|(_, outcome, _)| {
            matches!(
                (strength, outcome),
                (Strength::Review, Outcome::Review(_))
                    | (Strength::Consider, Outcome::Consider(_))
                    | (Strength::Note, Outcome::Note(_))
            )
        })
        .filter_map(|(question, _, leaned)| {
            VALUE_SIGNALS
                .iter()
                .find(|s| s.question == *question)
                .map(|s| (s, *leaned))
        })
        .collect();
    let reasons: Vec<String> = reached
        .iter()
        .map(|(signal, leaned)| match (strength, leaned) {
            (Strength::Note, true) => {
                format!("may {}; the answer was split", base_form(signal.finding))
            }
            (Strength::Note, false) => signal.note.to_string(),
            _ => signal.finding.to_string(),
        })
        .collect();
    let subject = if name == "module constants" {
        "Module constants".to_string()
    } else {
        format!("`{name}`")
    };
    let likely = if strength == Strength::Consider {
        " likely"
    } else {
        ""
    };
    (
        format!("{subject}{likely} {} ({p:.2}).", reasons.join("; ")),
        match (strength, reached.first()) {
            (Strength::Note, _) => "Optional: name or configure the value if it changes",
            (_, Some((signal, _))) => signal.action,
            (_, None) => "Review the values",
        },
    )
}

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
pub(super) fn section_wording(
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
    let directory = super::outcome::choice(get("scope")).map(|(dir, _)| dir);
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
        Some((heading, line)) if heading == super::instructions::PREAMBLE => {
            format!("The text at line {line}, before the first heading,")
        }
        Some((heading, line)) => format!("Section `{heading}` at line {line}"),
        None if name == super::instructions::PREAMBLE => format!("The {name}"),
        None => format!("Section `{name}`"),
    };
    let load = match detail {
        Detail::Section { tokens, loaded } if !loaded.is_empty() => {
            format!(" {loaded} (about {tokens} tokens).")
        }
        _ => String::new(),
    };
    (
        format!("{subject} {} ({p:.2}).{load}", reasons.join("; ")),
        match (strength, reached.first()) {
            (Strength::Note, Some(signal)) if signal.question == "scope" => signal.action,
            (Strength::Note, _) => "Optional: trim what agents learn from the code",
            (_, Some(signal)) => signal.action,
            (_, None) => "Review the section",
        },
    )
}

/// Each test-value signal that reached review, in plain words.
pub(super) fn test_wording(name: &str, review: bool, p: f64, answers: &Answers<'_>) -> Wording {
    let reasons: Vec<&str> = [
        (
            "own_logic",
            "computes its expected value with the logic it tests",
        ),
        (
            "mock_only",
            "only checks values its mocks were set to return",
        ),
        (
            "internal",
            "asserts internal details instead of observable results",
        ),
        ("several", "checks several unrelated behaviors"),
    ]
    .into_iter()
    .filter(|(q, _)| matches!(answers.get(q).map(|a| noul(a)), Some(Outcome::Review(_))))
    .map(|(_, text)| text)
    .collect();
    (
        format!("`{name}` {} ({p:.2}).", reasons.join("; ")),
        if review {
            "Assert on the behavior of the code under test with an independent expected value"
        } else {
            "Assert on observable results, one behavior per test"
        },
    )
}

pub(super) fn test_pair_wording(name: &str, review: bool, p: f64) -> Wording {
    if review {
        (
            format!(
                "{name} check the same behavior with equivalent inputs; one adds nothing ({p:.2})."
            ),
            "Remove one of the tests",
        )
    } else {
        (
            format!("{name} check the same behavior with different inputs ({p:.2})."),
            "Combine them into one parameterized test",
        )
    }
}

/// Injection kinds: the text a variable is placed into, its weakness and remedy.
const INJECTIONS: [(&str, &str, &str, &str); 7] = [
    (
        "sql",
        "a database query",
        "CWE-89 SQL injection",
        "Pass the values as bound query parameters",
    ),
    (
        "shell",
        "a command",
        "CWE-78 OS command injection",
        "Pass arguments as a list to the program, without a shell",
    ),
    (
        "code",
        "code it evaluates",
        "CWE-94 code injection",
        "Map the input to allowed operations instead of evaluating text built from it",
    ),
    (
        "markup",
        "markup",
        "CWE-79 cross-site scripting",
        "Escape the value or render it as text",
    ),
    (
        "path",
        "a file path",
        "CWE-22 path traversal",
        "Resolve the path and check that it stays under the allowed directory",
    ),
    (
        "url",
        "a URL it requests",
        "CWE-918 server-side request forgery",
        "Check the host against an allowed list before requesting it",
    ),
    (
        "",
        "text another program interprets",
        "CWE-74 injection",
        "Pass the value as data, not as part of the text",
    ),
];

/// Weak settings: what the code does, its weakness and remedy.
const SETTINGS: [(&str, &str, &str, &str); 6] = [
    (
        "tls",
        "turns off certificate or signature verification",
        "CWE-295 improper certificate validation",
        "Keep verification on; trust a specific certificate authority instead",
    ),
    (
        "hash",
        "hashes passwords with a fast or broken hash",
        "CWE-916 weak password hash",
        "Hash passwords with Argon2, bcrypt or scrypt",
    ),
    (
        "random",
        "makes secret tokens or identifiers with a non-cryptographic random generator",
        "CWE-338 weak random for secrets",
        "Use a cryptographically secure random generator",
    ),
    (
        "cors",
        "allows credentialed requests from any origin",
        "CWE-942 permissive CORS",
        "Allow only the origins that need credentialed access",
    ),
    (
        "cookie",
        "sets session cookies without HttpOnly, Secure or SameSite",
        "CWE-1004 cookie without HttpOnly or Secure",
        "Set HttpOnly, Secure and SameSite on session cookies",
    ),
    (
        "",
        "chooses a weak security setting",
        "CWE-1188 insecure setting",
        "Use the secure default",
    ),
];

/// The specific check of a rule's trace that found the concern most surely.
fn found_check(rule: &str, answers: &Answers<'_>) -> &'static str {
    super::security::checks(rule)
        .iter()
        .filter_map(|check| match answers.get(check.id).map(|a| noul(a)) {
            Some(Outcome::Review(p)) => Some((check.id, p)),
            _ => None,
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .or_else(|| {
            // A note from a leaning check names the kind it leaned toward.
            super::security::checks(rule)
                .iter()
                .filter_map(|check| match answers.get(check.id) {
                    Some(Answer::Noul { noul })
                        if crate::policy::probability_at_least(
                            *noul,
                            crate::policy::LEADING_PROBABILITY,
                        ) =>
                    {
                        Some((check.id, *noul))
                    }
                    _ => None,
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
        })
        .map_or("", |(id, _)| id)
}

/// A security finding's message, action and category (a CWE and its name).
pub(super) fn security_wording(
    rule: &str,
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let subject = if name == "module setup" {
        "Module setup".to_string()
    } else {
        format!("`{name}`")
    };
    let kind = found_check(rule, answers);
    match rule {
        catalog::INJECTION => injection_wording(&subject, kind, strength, p, answers),
        catalog::SENSITIVE_DATA => {
            let (what, category, action) = exposure_kind(answers);
            exposure_wording(&subject, (what, category, action), strength, p, answers)
        }
        _ => {
            let (_, what, category, action) = kind_row(&SETTINGS, kind);
            exposure_wording(&subject, (what, category, action), strength, p, answers)
        }
    }
}

/// The row of a kind table for the check that found the concern; the last
/// row is the general case.
fn kind_row(
    table: &'static [(&'static str, &'static str, &'static str, &'static str)],
    kind: &str,
) -> &'static (&'static str, &'static str, &'static str, &'static str) {
    table
        .iter()
        .find(|(id, ..)| *id == kind)
        .unwrap_or(table.last().unwrap())
}

fn injection_wording(
    subject: &str,
    kind: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let (_, noun, category, action) = kind_row(&INJECTIONS, kind);
    let outside = matches!(
        answers.get("origin").map(|a| origin_outcome(a)),
        Some(Outcome::Review(_))
    );
    let message = match (strength, outside) {
        (Strength::Review, _) => format!(
            "{subject} places values from another party into {noun} without binding, escaping or checking them ({p:.2})."
        ),
        (Strength::Consider, true) => format!(
            "{subject} places values from another party into {noun}; they may not be bound, escaped or checked ({p:.2})."
        ),
        (Strength::Consider, false) => format!(
            "{subject} places its parameters into {noun} without binding, escaping or checking them; a caller passing outside input would make it exploitable ({p:.2})."
        ),
        (Strength::Note, true) => format!(
            "{subject} places values from another party into {noun}, but no check found one placed unhandled ({p:.2})."
        ),
        (Strength::Note, false) => format!(
            "{subject} places a parameter into {noun}; it may already be bound or checked, or its callers may pass only the program's own values ({p:.2})."
        ),
    };
    let action = if strength == Strength::Note {
        "Optional: bind or check the value where it enters"
    } else {
        action
    };
    ((message, action), category.to_string())
}

/// Logging or error details, whichever signal is strongest.
fn exposure_kind(answers: &Answers<'_>) -> (&'static str, &'static str, &'static str) {
    let strongest = |questions: &[&str]| {
        questions
            .iter()
            .filter_map(|q| match answers.get(q) {
                Some(Answer::Noul { noul }) => Some(*noul),
                _ => None,
            })
            .fold(0.0, f64::max)
    };
    if strongest(&["logs_secret", "logs_object_secret"])
        >= strongest(&["error_details", "exception_to_client"])
    {
        (
            "writes a password, token, key or personal data to a log",
            "CWE-532 sensitive data in logs",
            "Log an identifier instead of the secret or personal value",
        )
    } else {
        (
            "sends internal error details to a remote client",
            "CWE-209 error details exposed",
            "Return a generic message and keep the details in server logs",
        )
    }
}

/// A logged secret, exposed details or weak setting: one level lower and so
/// marked when it runs only in development; a note comes from an answer that
/// leaned toward the concern without deciding it.
fn exposure_wording(
    subject: &str,
    (what, category, action): (&str, &'static str, &'static str),
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let development = matches!(
        answers.get("dev_only").map(|a| noul(a)),
        Some(Outcome::Review(_))
    );
    let where_ = if development {
        " It runs only in development or tests."
    } else {
        ""
    };
    let message = match strength {
        Strength::Note => format!(
            "{subject} may {}; the answer was split ({p:.2}).{where_}",
            base_form(what)
        ),
        Strength::Consider => format!("{subject} likely {what} ({p:.2}).{where_}"),
        Strength::Review => format!("{subject} {what} ({p:.2}).{where_}"),
    };
    ((message, action), category.to_string())
}

/// A phrase whose first word is a present-tense verb, after "may":
/// "sends details" becomes "send details", "hashes passwords" "hash passwords".
fn base_form(phrase: &str) -> String {
    let (verb, rest) = phrase.split_once(' ').unwrap_or((phrase, ""));
    let base = ["shes", "ches", "sses", "xes"]
        .iter()
        .find(|ending| verb.ends_with(*ending))
        .map_or_else(
            || verb.strip_suffix('s').unwrap_or(verb),
            |_| &verb[..verb.len() - 2],
        );
    if rest.is_empty() {
        base.to_string()
    } else {
        format!("{base} {rest}")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn verbs_after_may_take_their_base_form() {
        for (phrase, expected) in [
            (
                "sends internal error details",
                "send internal error details",
            ),
            ("hashes passwords", "hash passwords"),
            (
                "chooses a weak security setting",
                "choose a weak security setting",
            ),
            ("fixes a value", "fix a value"),
            ("uses a value", "use a value"),
            (
                "special-cases one specific identity",
                "special-case one specific identity",
            ),
        ] {
            assert_eq!(super::base_form(phrase), expected);
        }
    }
}
