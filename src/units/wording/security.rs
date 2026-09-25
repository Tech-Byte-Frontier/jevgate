//! Messages of the security, access-control and workflow rules, each naming its weakness.
use super::*;

/// Each access-control and workflow question: what a finding says the unit
/// does, the weakness it names, and the next step.
const PRIVILEGE: [(&str, &str, &str, &str); 7] = [
    (
        "others",
        "lets every user it applies to read or change other users' rows",
        "CWE-863 incorrect authorization",
        "Tie the condition to the user's id, account or membership",
    ),
    (
        "editable",
        "trusts a value users can change, such as `user_metadata`",
        "CWE-639 authorization through a user-controlled key",
        "Base access on the user id or on claims only the server sets, such as `app_metadata`",
    ),
    (
        "unchecked",
        "reads or changes other users' rows without checking the caller",
        "CWE-862 missing authorization",
        "Check `auth.uid()` or a role in the function, or make it SECURITY INVOKER",
    ),
    (
        "search_path",
        "runs with its owner's privileges without a fixed `search_path`",
        "CWE-426 untrusted search path",
        "Add `set search_path = ''` and schema-qualify the names it uses",
    ),
    (
        "broad",
        "gives anon, public or every signed-in user more than reads of public data",
        "CWE-732 incorrect permission assignment",
        "Grant only what clients need, and enable row-level security on the table",
    ),
    (
        "outside",
        "places text that people outside the repository write into a `run` script",
        "CWE-78 command injection",
        "Pass the value through an `env` variable and quote it in the script",
    ),
    (
        "untrusted",
        "runs pull request code while it has secrets or a write token",
        "CWE-829 untrusted code with privileges",
        "Run untrusted code on `pull_request`, or keep secrets and write tokens out of the job that checks it out",
    ),
];

/// The finding of an access-control or workflow unit: the strongest question
/// that reached the concern names what it does and its weakness.
pub(in crate::units) fn privilege_wording(
    subject: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let reached = |q: &str| matches!(answers.get(q).map(|a| noul(a)), Some(Outcome::Review(_)));
    let (_, what, category, action) = PRIVILEGE
        .iter()
        .find(|(q, ..)| reached(q))
        .copied()
        .unwrap_or(PRIVILEGE[0]);
    let message = match strength {
        Strength::Review => format!("{subject} {what} ({p:.2})."),
        _ => format!("{subject} likely {what} ({p:.2})."),
    };
    ((message, action), category.to_string())
}

/// Injection kinds: the text a variable is placed into, its weakness and remedy.
const INJECTIONS: [(&str, &str, &str, &str); 8] = [
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
        "type",
        "the types of objects it creates or deserializes",
        "CWE-502 deserialization of untrusted data",
        "Create and deserialize only types fixed in the code or on an allowed list",
    ),
    (
        "",
        "text another program interprets",
        "CWE-74 injection",
        "Pass the value as data, not as part of the text",
    ),
];

/// Weak settings: what the code does, its weakness and remedy.
const SETTINGS: [(&str, &str, &str, &str); 9] = [
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
        "makes secret tokens or identifiers that can be guessed, with a non-cryptographic random generator or from known data",
        "CWE-330 insufficiently random values",
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
        "debug",
        "shows detailed error pages or debugging tools outside development",
        "CWE-489 active debug code",
        "Turn detailed errors and debug tools on only in the development environment",
    ),
    (
        "token",
        "accepts tokens without checking their signature or expiry",
        "CWE-347 improper verification of cryptographic signature",
        "Validate each token's signature and lifetime",
    ),
    (
        "key",
        "signs or encrypts with a key written in the code",
        "CWE-321 hard-coded cryptographic key",
        "Read the key from configuration or a secret store, and replace the one in the code",
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
    crate::units::security::checks(rule)
        .iter()
        .filter_map(|check| match answers.get(check.id).map(|a| noul(a)) {
            Some(Outcome::Review(p)) => Some((check.id, p)),
            _ => None,
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .or_else(|| {
            // A note from a leaning check names the kind it leaned toward.
            crate::units::security::checks(rule)
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
pub(in crate::units) fn security_wording(
    rule: &str,
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let subject = if name == crate::units::security::MODULE_SETUP {
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
            "{subject} places values from another party into {noun}, but no check found one placed unhandled."
        ),
        (Strength::Note, false) => format!(
            "{subject} places a parameter into {noun}; it may already be bound or checked, or its callers may pass only the program's own values."
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
    let get = |q: &str| answers.get(q).copied();
    let foreign = category.starts_with("CWE-209")
        && matches!(
            crate::units::outcome::messages(&get),
            Some(crate::units::outcome::Messages::Foreign(_))
        );
    let message = match strength {
        Strength::Note => format!(
            "{subject} may {}; the answer was split.{where_}",
            base_form(what)
        ),
        Strength::Consider if foreign => format!(
            "{subject} puts the text of a library or database error into an error message ({p:.2}), which likely reaches a remote client.{where_}"
        ),
        Strength::Consider => format!("{subject} likely {what} ({p:.2}).{where_}"),
        Strength::Review => format!("{subject} {what} ({p:.2}).{where_}"),
    };
    ((message, action), category.to_string())
}

/// A SpacetimeDB table, view or reducer: what it lets a client do, the
/// weakness it names and the next step, from the check that reached review.
pub(in crate::units) fn module_wording(
    access: &crate::units::Access,
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let reached = |q: &str| matches!(answers.get(q).map(|a| noul(a)), Some(Outcome::Review(_)));
    let (subject, what, category, action) = match access {
        crate::units::Access::Table => (
            format!("Public table `{name}`"),
            "lets every client read rows that hold individual users' data",
            "CWE-359 exposure of private personal information",
            "Make the table private and give each user their own rows through a view that filters by `ctx.sender`",
        ),
        crate::units::Access::View => (
            format!("View `{name}`"),
            "returns rows of other users or accounts without limiting them to the caller",
            "CWE-863 incorrect authorization",
            "Limit the rows to the caller through `ctx.sender`, or return only data meant for every user",
        ),
        _ if reached("argument_rows") => (
            format!("Reducer `{name}`"),
            "reads or changes a row its arguments choose without confirming the row belongs to the caller",
            "CWE-639 authorization through a user-controlled key",
            "Find the row through `ctx.sender`, or check that it belongs to the caller before using it",
        ),
        _ if reached("operator_only") => (
            format!("Reducer `{name}`"),
            "changes settings, shared content or another account without checking that the caller may",
            "CWE-862 missing authorization",
            "Require the module owner, an admin or the role that grants the change before making it",
        ),
        _ => (
            format!("Reducer `{name}`"),
            "lets a client change other users' state or admin-only settings",
            "CWE-862 missing authorization",
            "Check the caller before the change",
        ),
    };
    let likely = if strength == Strength::Review {
        ""
    } else {
        " likely"
    };
    (
        (
            format!("{subject}{likely} {what}{}.", shown(strength, p)),
            action,
        ),
        category.to_string(),
    )
}

/// An error handler that sends clients more than the program's own messages.
pub(in crate::units) fn handler_wording(
    name: &str,
    registered: &str,
    strength: Strength,
    p: f64,
) -> Wording {
    let likely = if strength == Strength::Review {
        ""
    } else {
        " likely"
    };
    (
        format!(
            "`{name}`, the error handler registered by {registered},{likely} sends clients more than the program's own error messages and codes, such as another error's text, its cause or its stack{}.",
            shown(strength, p)
        ),
        "Send only the program's own messages and codes, and a fixed message for any other error; keep details in server logs",
    )
}
