//! Questions about security in application code: whether a function places
//! variables into interpreted text, logs secrets or weakens a setting, and the
//! literal checks per kind that the trace and caller rechecks ask.
use super::{EVIDENCE, choose_id, noul, score};
use serde_json::{Value, json};

/// Whether a function places a variable into text another program runs or
/// renders. Presence only: the trace questions decide whether it is a concern.
pub fn security_interpreted(code: &str) -> Value {
    noul(
        format!(
            "Does `{code}` place a variable into the text of a database query, shell command, code to evaluate, or HTML markup?"
        ),
        "A variable is joined, formatted or interpolated into the text of a query, command, code or markup that is then run or rendered.",
        "Variables are passed only as bound parameters, separate arguments, or through a template or component that escapes them; the text is built only from fixed values; or the function builds no such text.",
    )
}

pub fn security_resource(code: &str) -> Value {
    noul(
        format!(
            "Does `{code}` open, write or request a file path or URL that is taken or built from a variable?"
        ),
        "The path of a file it opens, writes or deletes, or the URL it requests, comes from or is built with a variable such as a parameter or a value read from input.",
        "Every path and URL it uses is fixed in the code or read from the program's configuration, or it uses none.",
    )
}

pub fn security_logs_secret(code: &str) -> Value {
    noul(
        format!("Does `{code}` write a password, token, key or personal data to a log or console?"),
        "It logs or prints a password, token, API key, session identifier, or personal data about a person such as an email address or document number.",
        "It logs only messages, identifiers that are not secret such as record ids, counts, or errors without such values, or it logs nothing.",
    )
}

/// An app's own error reporter is not a response: "in a response to a remote
/// client" alone matched a front end posting a stack to its own server.
pub fn security_error_details(code: &str) -> Value {
    noul(
        format!(
            "Does `{code}` put internal error details, such as stack traces, database errors or server paths, into the response it sends to a remote client?"
        ),
        "It puts an exception's stack trace, a database or library error message, a query, or a server path into the response to a request from a remote client.",
        "It returns generic messages or error codes, keeps details in server logs or sends them to its own error-reporting service, shows errors to the local user of a command-line or desktop program, or sends no response.",
    )
}

pub fn security_weakened(code: &str) -> Value {
    json!({
        "type": "noul",
        "instructions": {
            "question": format!("Does `{code}` turn off a security check or choose a weak security setting?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "true": {
                "what": "It weakens protection that the program relies on.",
                "examples": [
                    "Certificate or signature verification turned off",
                    "Passwords hashed with a fast or broken hash such as MD5 or SHA-1",
                    "Tokens, passwords or identifiers that must be unguessable made with a non-cryptographic random generator",
                    "Any origin allowed to send credentialed requests",
                    "Session cookies set without HttpOnly or Secure"
                ]
            },
            "false": {
                "what": "It uses secure defaults or strong settings, or uses such functions where security does not depend on them.",
                "examples": [
                    "Checksums, cache keys or content hashes",
                    "Shuffling, sampling or visual effects",
                    "Test data or local development tools"
                ]
            }
        },
    })
}

/// Where the variables of a query, command, markup, path or URL come from.
/// The middle level (parameters of unknown origin) is a concern for a
/// caller to settle, not a sign that the code is fine.
pub fn security_origin(code: &str, callers: bool) -> Value {
    score(
        format!(
            "Where do the variables that `{code}` places into a query, command, code, markup, file path or URL come from?"
        ),
        if callers { CALLERS } else { "" },
        [
            "Only from the program itself: constants, fixed choices, numbers, values checked against an allowed list, the deployment's configuration, or the command line and settings of the person running a local program.",
            "From the function's parameters or other data whose origin this code does not show.",
            "From another party: a network request, message, uploaded file, or a record other users can edit, used as received.",
        ],
    )
}

/// Asked in the sensitive-data trace: whether every error message is the
/// program's own. It can only clear the error-detail signals; functions that
/// throw the program's typed errors otherwise stayed undecided, since the
/// response is written elsewhere. A database error attached as the cause is
/// not message text: the error handler that writes responses is judged itself.
pub fn security_own_messages(code: &str) -> Value {
    noul(
        format!(
            "Is the message of every error that `{code}` throws, returns or sends text the program writes itself?"
        ),
        "Each message is the program's own fixed or formatted text, or it raises no errors. An exception may be attached as the cause of the program's own error.",
        "An exception's stack trace, a database or library error message, a query or a server path is part of an error message or of a response.",
    )
}

pub fn security_dev_only(code: &str) -> Value {
    noul(
        format!(
            "Does `{code}` run only in tests, local development tools or scripts a developer runs by hand?"
        ),
        "It is test code, a local development tool, or a script for the developer, not part of what is deployed or shipped to users.",
        "It is part of the application that is deployed or shipped to users.",
    )
}

/// Which site holds what a presence question found, to locate the finding.
pub fn security_site(what: &str, sites: &[String]) -> Value {
    choose_id(
        &format!("Which entry in `sites` {what}?"),
        format!(
            "Options are the `id` values in `sites`, each a statement of `function.source`. {EVIDENCE}"
        ),
        sites,
        "No single entry does.",
    )
}

/// One specific, literal check of a trace: its id (also the finding's kind),
/// question with `{code}` for the source path, and both answers.
pub struct Check {
    pub id: &'static str,
    question: &'static str,
    yes: &'static str,
    no: &'static str,
}

impl Check {
    pub fn body(&self, code: &str) -> Value {
        noul(self.question.replace("{code}", code), self.yes, self.no)
    }

    /// The same check with the functions that call the code in `callers`.
    pub fn with_callers(&self, code: &str) -> Value {
        let mut body = self.body(code);
        body["instructions"]["note"] = json!(format!("{CALLERS} {EVIDENCE}"));
        body
    }
}

const CALLERS: &str = "`callers` holds functions that call it.";

/// Whether a variable reaches each kind of interpreted text unhandled. The
/// path check names the program's own directories as safe: without them, a
/// third of injection units stayed undecided on paths the program builds from
/// its project root. One
/// broad question ("is every variable bound, escaped or checked?") stayed
/// undecided even for `eval` of model output; one literal check per kind
/// decides, and names the kind.
pub const UNHANDLED: [Check; 6] = [
    Check {
        id: "sql",
        question: "Does `{code}` put a variable into the text of an SQL query instead of passing it as a bound parameter?",
        yes: "A variable is joined, formatted or interpolated into SQL text that is then run.",
        no: "Values are passed as bound parameters or placeholders, identifiers come from a fixed list or are quoted by the database library, or it runs no SQL.",
    },
    Check {
        id: "shell",
        question: "Does `{code}` run a shell command string that holds a variable?",
        yes: "A command line built with a variable is run through a shell, such as with exec, execSync, os.system, subprocess with shell=True, or sh -c.",
        no: "It runs programs with a list of arguments and no shell, quotes each variable for the shell, or runs no command.",
    },
    Check {
        id: "code",
        question: "Does `{code}` evaluate text that holds a variable as code or as a template?",
        yes: "It passes text that holds a variable to eval, exec, new Function, a template compiler or a similar evaluator.",
        no: "It parses data with a data-only parser such as JSON or a literal parser, or evaluates only fixed code.",
    },
    Check {
        id: "markup",
        question: "Does `{code}` put a variable into HTML or SVG markup without escaping it?",
        yes: "A variable is joined into HTML or SVG text, or assigned to innerHTML or a similar raw-markup property, without an escaping function.",
        no: "Values go through an escaping function or a template or component that escapes them, or it builds no markup.",
    },
    Check {
        id: "path",
        question: "Does `{code}` open, write or delete a file at a path built from a variable without checking that it stays inside a directory?",
        yes: "A path is built from a variable that can hold a name or path from outside the program, such as a request, upload, archive entry or user input, and is used without reducing it to a base name, rejecting parent-directory parts, or checking that the resolved path stays under a base directory.",
        no: "Such paths are checked; are built from the program's own directories, such as its project root, data or cache directory, joined with names the program chooses; come from the program's configuration or the command line of the person running it; or it uses no such path.",
    },
    Check {
        id: "url",
        question: "Does `{code}` request a URL or host taken from a variable without checking the host?",
        yes: "It sends a request to a URL or host that comes from a variable, without checking the host against an allowed list or rejecting private addresses.",
        no: "The host is fixed, comes from the program's configuration, or is checked, or it requests no URL.",
    },
];

/// Specific weak settings, asked when the broad presence question is not clear.
pub const WEAK_SETTINGS: [Check; 5] = [
    Check {
        id: "tls",
        question: "Does `{code}` turn off certificate or host name verification?",
        yes: "It turns off certificate or host name checks, or accepts invalid certificates or host names.",
        no: "It keeps verification on, or makes no TLS connection.",
    },
    Check {
        id: "hash",
        question: "Does `{code}` hash passwords or derive keys from them with a fast or broken hash, or with few iterations?",
        yes: "It hashes passwords or derives keys from them with MD5, SHA-1, a single round of SHA-256, or a key derivation function with few iterations.",
        no: "It uses bcrypt, scrypt, Argon2 or a key derivation function with many iterations, or it does not handle passwords.",
    },
    Check {
        id: "random",
        question: "Does `{code}` make a token, code, password or identifier that must be unguessable with a non-cryptographic random generator?",
        yes: "It uses a generator such as Math.random or Python's random module for a secret value, such as a session token, reset or verification code, or random password.",
        no: "It uses a cryptographic generator such as crypto.randomUUID, secrets or OsRng, or the random value is not a secret.",
    },
    Check {
        id: "cors",
        question: "Does `{code}` let pages from origins it does not fully check read its responses with credentials?",
        yes: "It allows any origin, reflects the request's origin, or matches origins loosely, such as by suffix or substring, while allowing credentials.",
        no: "It allows only listed origins by exact match, allows no credentials, or sets no cross-origin rules.",
    },
    Check {
        id: "cookie",
        question: "Does `{code}` set or configure a session or authentication cookie without the Secure or HttpOnly flag?",
        yes: "A cookie that holds a session or token is set or configured without Secure or without HttpOnly.",
        no: "Such cookies have both flags, the cookie holds no session or token, or the code sets no cookie.",
    },
];

/// Specific exposures, asked when the broad presence questions are not clear:
/// a logged configuration or argument list that holds a password, and an
/// exception's own text in a response, were left undecided by them.
pub const EXPOSURES: [Check; 2] = [
    Check {
        id: "logs_object_secret",
        question: "Does `{code}` log an object, configuration, request, command or list of arguments that holds a password, token or key?",
        yes: "It logs a whole object, configuration, request, command line or argument list, and that value holds a password, token, key or secret.",
        no: "It logs only values that hold no secret, masks secrets before logging, or logs nothing.",
    },
    Check {
        id: "exception_to_client",
        question: "Does `{code}` send an exception's message, stack trace or a database error to a remote client in a response?",
        yes: "The text of an exception it did not raise itself to explain bad input, or a stack trace, is put into the response to a request.",
        no: "Responses carry fixed messages or codes, or only messages the program wrote to explain invalid input; details stay in server logs.",
    },
];
