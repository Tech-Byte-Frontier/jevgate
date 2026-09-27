//! The Choices a security finding is asked after it is raised: what the values an
//! injection places can hold, what its paths can hold, what its markup holds
//! and where its redirects lead, and when a log line runs.
use super::*;

/// What the values of an injection consider resting on the function's
/// parameters can hold, asked only for such a finding, with its callers.
/// Labeled by hand, those considers were right 30 times in 81: most wrong
/// ones placed text every caller passes as a literal, such as a Rust
/// helper's SQL fragments, or a command-line tool's own arguments, while
/// right ones placed names from a database others write, fetched page
/// titles or model output. Asked where the values come from, the recheck
/// answered "the function's parameters" at 0.9 even when its callers passed
/// literals.
pub fn injection_values(code: &str, callers: bool) -> Value {
    let (fixed, note) = if callers {
        (
            "Only text the program fixes: literals and constants, written in this code or passed by every caller in `callers`; numbers, dates or other typed values that cannot hold markup or syntax; or names chosen from a fixed list.",
            format!("{CALLERS} {EVIDENCE}"),
        )
    } else {
        (
            "Only text the program fixes: literals and constants written in this code; numbers, dates or other typed values that cannot hold markup or syntax; or names chosen from a fixed list.",
            EVIDENCE.to_string(),
        )
    };
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("What can the values that `{code}` places into a query, command, code or markup without binding or escaping them hold?"),
            "note": note,
        },
        "criteria": {
            "fixed": fixed,
            "own": "Values the program creates or keeps for itself, such as ids it generates, the names of its own tables, files or settings, or text it wrote itself.",
            "local": "The arguments of a command-line program, build script or code generator, typed by the person who runs it on their own machine, or text that person runs on purpose, such as a query they typed.",
            "outside": "Text another party can set: a network request, message or uploaded file, a page or feed fetched from the network, a language model's output, or records and names other users can write, such as rows of a shared database.",
            "unknown": "Values from parameters or calls whose origin is not shown, which may hold any of these.",
        },
    })
}

/// The options of `injection_values` that hold only the program's own values.
pub const PROGRAM_VALUES: [&str; 3] = ["fixed", "own", "local"];

/// What the variable parts of a path finding's paths can hold, asked only
/// for an injection finding whose check found a path, with its callers and
/// the definitions of the project's types its parameters name. On
/// vaultwarden, Rocket route parameters typed `PathBuf` (which Rocket parses
/// so they cannot climb above where they are joined) and id types whose
/// parsing accepts only a UUID were four wrong path reviews: the path check
/// reads a variable joined to a directory, whatever the variable can hold.
pub fn injection_paths(code: &str, callers: bool, types: bool) -> Value {
    let types_note = if types {
        " `types_named_in_parameters` holds the definitions of the project's types that its parameters name, with their attributes."
    } else {
        ""
    };
    let lead = if callers {
        format!("{CALLERS}{types_note}")
    } else {
        types_note.trim_start().to_string()
    };
    let note = if lead.is_empty() {
        EVIDENCE.to_string()
    } else {
        format!("{lead} {EVIDENCE}")
    };
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("What can the variable parts of the file paths that `{code}` opens, writes or deletes hold?"),
            "note": note,
        },
        "criteria": {
            "confined": "Only names that cannot leave the directory they are joined to: numbers, UUIDs or ids that a type or the web framework parses before the function runs, names reduced to a base name or checked against a pattern, or a path parameter the framework parses so it cannot climb above where it is joined, such as a Rocket `PathBuf` route segment, which rejects hidden and encoded-slash segments and drops `..` at its start.",
            "own": "Names the program chooses or keeps for itself, or reads from its configuration.",
            "local": "The command line, settings or files of the person running a local program or script.",
            "outside": "A name or path another party sets that can hold `..`, a slash or an absolute path, such as a request parameter or field read as text, an uploaded file's name or an archive entry.",
            "unknown": "Values from parameters or calls whose origin is not shown, which may hold any of these.",
        },
    })
}

/// The options of `injection_paths` that keep a path inside its directory.
pub const CONFINED_PATHS: [&str; 3] = ["confined", "own", "local"];

/// What a markup finding's values hold where they enter the markup, asked
/// only after a finding whose one concern is markup. vaultwarden's
/// `hibp_breach` percent-encodes the username before it builds the link,
/// oak's examples write a URL object whose serialization percent-encodes
/// `<` and `>`, and a JSP page runs its own `esc()` first: the markup check
/// reads a variable joined into HTML, whatever it was turned into before.
pub fn markup_values(code: &str, callers: bool) -> Value {
    let (by_callers, note) = if callers {
        (
            " or by the functions in `callers`",
            format!("{CALLERS} {EVIDENCE}"),
        )
    } else {
        ("", EVIDENCE.to_string())
    };
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("What do the values that `{code}` places into HTML or SVG markup hold where they enter it?"),
            "note": note,
        },
        "criteria": {
            "encoded": format!("Text already escaped for HTML, percent-encoded or serialized as a URL before it enters the markup, in this code{by_callers}, so it cannot hold `<`, `>`, `&` or quotes."),
            "typed": "Numbers, dates, booleans or ids, or names chosen from a fixed list.",
            "own": "Text the program writes itself or reads from its configuration.",
            "raw": "Text as another party or a caller wrote it, which can hold `<`, `>`, `&` or quotes.",
            "unknown": "Values whose origin or handling is not shown.",
        },
    })
}

/// The options of `markup_values` that cannot open a tag or attribute.
pub const HARMLESS_MARKUP: [&str; 3] = ["encoded", "typed", "own"];

/// Where a redirect finding's targets can lead, asked only after a finding
/// whose one concern is a redirect. vaultwarden's admin login redirects to
/// its admin path followed by the form's value, and shiori's to its login
/// page with the current path as a query value: a fixed path before the
/// variable keeps the target on the site, which the redirect check does not
/// ask. Offered "its own origin and a slash" without the form written out,
/// chatbot-ui's `requestUrl.origin + next` read as staying on the site at
/// 0.63, though `next=@evil.com` leaves it.
pub fn redirect_reach(code: &str, callers: bool) -> Value {
    let note = if callers {
        format!("{CALLERS} {EVIDENCE}")
    } else {
        EVIDENCE.to_string()
    };
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("Where can the targets that `{code}` redirects clients to lead?"),
            "note": note,
        },
        "criteria": {
            "own_site": "Only to the program's own site: every target starts with a fixed path written in the code, such as `/admin` or `/login?next=`, so it begins with one slash and a path; or with the program's own origin followed by a slash written in the code; variables only follow that fixed part or fill its query string.",
            "checked": "Only where a check allows: the target is compared with an allowed list of hosts or checked to be a path on the site before the redirect.",
            "anywhere": "Anywhere a variable says: a variable starts the target, or directly follows the program's own origin or a host with no slash written between them, as in `origin + next`, where `@evil.com` or `.evil.com` in the variable names another host.",
            "none": "It does not redirect.",
        },
    })
}

/// The options of `redirect_reach` that keep a redirect on the site.
pub const OWN_SITE: [&str; 3] = ["own_site", "checked", "none"];

/// When a logging finding's log line runs, asked only for a sensitive-data
/// finding raised by its log checks. vaultwarden logs SSO tokens inside
/// `if CONFIG.sso_debug_tokens()`, a setting off by default and documented
/// for logging them while troubleshooting: logging an identifier instead,
/// as the finding says, would remove the feature.
pub fn logged_when(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("When does `{code}` write the secret or personal value to a log?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "always": "Whenever that code runs, at a level the program logs at in normal operation, such as info, warning or error.",
            "debug": "Only at debug or trace level, which an operator may turn on to troubleshoot.",
            "opt_in": "Only when an operator turns on a setting, off by default, whose purpose is to log these values for troubleshooting, such as an option named for logging tokens or request bodies.",
            "none": "It writes no secret or personal value to a log.",
        },
    })
}

/// The option of `logged_when` for a setting whose purpose is the logging.
pub const OPT_IN_LOGGING: &str = "opt_in";
