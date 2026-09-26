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
        "reads or changes other users' rows or files without checking the caller",
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
const INJECTIONS: [(&str, &str, &str, &str); 12] = [
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
        "redirect",
        "a URL it redirects clients to",
        "CWE-601 open redirect",
        "Redirect only to paths on this site or to hosts on an allowed list",
    ),
    (
        "deserialize",
        "a deserializer that can build any object",
        "CWE-502 deserialization of untrusted data",
        "Parse the data as JSON or with a safe loader such as `yaml.safe_load`, or restrict the classes it may create",
    ),
    (
        "xxe",
        "an XML parser that resolves external entities",
        "CWE-611 XML external entity reference",
        "Turn off document type definitions and external entities in the parser, or parse with one that never resolves them",
    ),
    (
        "upload",
        "the name of a file it saves",
        "CWE-434 unrestricted file upload",
        "Allow only listed extensions and name saved uploads yourself",
    ),
    (
        "",
        "text another program interprets",
        "CWE-74 injection",
        "Pass the value as data, not as part of the text",
    ),
];

/// Weak settings: what the code does, its weakness and remedy.
const SETTINGS: [(&str, &str, &str, &str); 13] = [
    (
        "tls",
        "turns off certificate or signature verification",
        "CWE-295 improper certificate validation",
        "Keep verification on; trust a specific certificate authority instead",
    ),
    (
        "hash",
        "keeps passwords as plain text or hashes them with a fast or broken hash",
        "CWE-256 plaintext password or CWE-916 weak password hash",
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
        "public_secret",
        "reads a secret from an environment variable the build puts into browser code",
        "CWE-200 secret exposed to browsers",
        "Read the secret from a variable without the public prefix, only in server code, and rotate it",
    ),
    (
        "escape",
        "turns off the escaping of values written into HTML",
        "CWE-79 cross-site scripting",
        "Keep automatic escaping on and mark only values that are already safe HTML as raw",
    ),
    (
        "csrf",
        "turns off cross-site request forgery protection for requests that change data",
        "CWE-352 cross-site request forgery",
        "Keep CSRF protection on and send the token with forms and scripts instead of exempting the view",
    ),
    (
        "literal_secret",
        "keeps a secret key, password or token as a literal in the code",
        "CWE-798 hard-coded credentials",
        "Read the secret from the environment or a secret store, and replace the committed value",
    ),
    (
        "",
        "chooses a weak security setting",
        "CWE-1188 insecure setting",
        "Use the secure default",
    ),
];

/// The specific checks of a rule's trace that found the concern, most surely
/// first: a PHP page script often builds a query and markup from the same
/// request, and naming only the strongest hid the other. Without one, the
/// check a note leaned toward. A check its settle Choice cleared is not named.
fn found_checks(rule: &str, answers: &Answers<'_>) -> Vec<&'static str> {
    let get = |q: &str| answers.get(q).copied();
    let mut found: Vec<(&'static str, f64)> = settled_checks(rule, &get)
        .into_iter()
        .filter_map(|(id, outcome)| match outcome {
            Outcome::Review(p) => Some((id, p)),
            _ => None,
        })
        .collect();
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    if !found.is_empty() {
        return found.into_iter().map(|(id, _)| id).collect();
    }
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
        .map(|(id, _)| id)
        .into_iter()
        .collect()
}

/// `a`, `a and b`, or `a, b and c`.
fn listed(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

type KindRow = (&'static str, &'static str, &'static str, &'static str);

/// The row of the strongest check found (the general row when none was),
/// with the phrases of every found check listed.
fn kind_rows(table: &'static [KindRow], kinds: &[&str]) -> (String, &'static KindRow) {
    let strongest = kind_row(table, kinds.first().copied().unwrap_or(""));
    let phrases: Vec<&str> = kinds.iter().map(|k| kind_row(table, k).1).collect();
    let phrase = if phrases.len() > 1 {
        listed(&phrases)
    } else {
        strongest.1.to_string()
    };
    (phrase, strongest)
}

/// How findings name a PHP file's top-level code.
const SCRIPT_SUBJECT: &str = "Top-level code";

/// A security finding's message, action and category (a CWE and its name).
pub(in crate::units) fn security_wording(
    rule: &str,
    name: &str,
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let subject = match name {
        crate::units::security::MODULE_SETUP => "Module setup".to_string(),
        crate::units::security::SETTINGS_MODULE => "Settings module".to_string(),
        crate::units::security::SCRIPT => SCRIPT_SUBJECT.to_string(),
        _ => format!("`{name}`"),
    };
    let kinds = found_checks(rule, answers);
    match rule {
        catalog::INJECTION => injection_wording(&subject, &kinds, strength, p, answers),
        catalog::SENSITIVE_DATA => {
            let (what, category, action) = exposure_kind(answers);
            exposure_wording(&subject, (what, category, action), strength, p, answers)
        }
        _ => {
            // The strongest weak setting names the finding; the others it
            // found are listed after it.
            let kind = kinds.first().copied().unwrap_or("");
            let (_, what, category, action) = kind_row(&SETTINGS, kind);
            let ((message, action), category) =
                exposure_wording(&subject, (what, category, action), strength, p, answers);
            ((message + &also_found(kind, answers), action), category)
        }
    }
}

/// The other weak settings a check found at the threshold, such as a weak
/// password hash beside debug mode in one settings module: the unit is one
/// finding, so its message names them all.
fn also_found(kind: &str, answers: &Answers<'_>) -> String {
    let others: Vec<&str> = SETTINGS
        .iter()
        .filter(|(id, ..)| !id.is_empty() && *id != kind)
        .filter(|(id, ..)| matches!(answers.get(id).map(|a| noul(a)), Some(Outcome::Review(_))))
        .map(|(_, _, category, _)| *category)
        .collect();
    if others.is_empty() {
        String::new()
    } else {
        format!(" Also found: {}.", others.join("; "))
    }
}

/// The row of a kind table for the check that found the concern; the last
/// row is the general case.
fn kind_row(table: &'static [KindRow], kind: &str) -> &'static KindRow {
    table
        .iter()
        .find(|(id, ..)| *id == kind)
        .unwrap_or(table.last().unwrap())
}

fn injection_wording(
    subject: &str,
    kinds: &[&str],
    strength: Strength,
    p: f64,
    answers: &Answers<'_>,
) -> (Wording, String) {
    let (noun, (_, _, category, action)) = kind_rows(&INJECTIONS, kinds);
    let outside = matches!(
        answers.get("origin").map(|a| origin_outcome(a)),
        Some(Outcome::Review(_))
    );
    // A page script has no parameters: what it does not show the origin of
    // is set by the files it includes or returned by the helpers it calls.
    if subject == SCRIPT_SUBJECT && !outside && strength != Strength::Review {
        let message = if strength == Strength::Consider {
            format!(
                "{subject} places values whose origin it does not show, such as those an included file sets or a helper returns, into {noun} without binding, escaping or checking them; outside input reaching them would make it exploitable ({p:.2})."
            )
        } else {
            format!(
                "{subject} places a value whose origin it does not show into {noun}; it may already be bound or checked, or hold only the program's own values."
            )
        };
        let action = if strength == Strength::Note {
            "Optional: bind or check the value where it enters"
        } else {
            action
        };
        return ((message, action), category.to_string());
    }
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
    let logs = strongest(&["logs_secret", "logs_object_secret"]);
    let errors = strongest(&["error_details", "exception_to_client"]);
    let environment = strongest(&["environment_to_client"]);
    if environment > logs && environment > errors {
        (
            "sends the server's environment, settings or request metadata to a remote client",
            "CWE-497 exposure of system data",
            "Send only the fields the client needs; keep the environment and settings on the server",
        )
    } else if logs >= errors {
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
            Some(crate::units::outcome::Messages::Foreign)
        );
    let decided = crate::policy::probability_at_least(p, crate::policy::REVIEW_PROBABILITY);
    let message = match strength {
        // A decided finding lowered because the code runs only in
        // development or tests; its answer was not split.
        Strength::Note if decided && development => {
            format!("{subject} {what}, but it runs only in development or tests.")
        }
        // A weak setting the presence answer found but no specific check
        // named, as in a settings module.
        Strength::Note if decided && category == "CWE-1188 insecure setting" => format!(
            "{subject} may {}; no specific check named the setting.",
            base_form(what)
        ),
        Strength::Note if foreign => format!(
            "{subject} puts the text of a library or database error into an error message, which may reach a remote client.{where_}"
        ),
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
