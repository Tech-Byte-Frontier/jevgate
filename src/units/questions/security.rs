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
            "Does `{code}` open, write, request or redirect to a file path or URL that is taken or built from a variable?"
        ),
        "The path of a file it opens, writes or deletes, the URL it requests, or the URL or path it redirects the client to, comes from or is built with a variable such as a parameter or a value read from input.",
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
                    "Session cookies set without HttpOnly or Secure",
                    "A secret key read from an environment variable whose prefix, such as NEXT_PUBLIC_, makes the build put it into browser code"
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
            "From another party: a network request, message, uploaded file, or a record other users can edit, used as received, including the arguments of a function that clients call directly as an endpoint.",
        ],
    )
}

/// Options of the URL-parts Choice that rule a URL concern out: a host of the
/// program's own, or no request.
pub const OWN_PARTS: [&str; 2] = ["own", "none"];

/// Where the URLs a function requests come from, asked when the URL check
/// stays undecided: on clients of a fixed or configured service the check
/// split on a variable path or query, while naming the host decided them. A
/// host that is sent another URL to fetch is its own option, since internal
/// proxies fetched what users sent. The same question about paths cleared
/// real traversals, reading names stored in an index as the program's own,
/// so paths are not settled this way.
pub fn security_url_parts(code: &str, callers: bool) -> Value {
    let shown = if callers {
        ", in the function or in what `callers` pass it"
    } else {
        ""
    };
    let note = if callers {
        format!("{CALLERS} {EVIDENCE}")
    } else {
        EVIDENCE.to_string()
    };
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("Where do the URLs that `{code}` requests come from?"),
            "note": note,
        },
        "criteria": {
            "own": format!("A host written in the code or set in the program's configuration or environment, with only ids, names, numbers or search terms from variables in its path or query{shown}."),
            "forwards": "A host from the code or configuration, with another URL or host from a variable passed in its path or query for that service to fetch.",
            "given": "A whole URL or host handed to the function as a parameter or field.",
            "outside": "A URL or host from outside the program, such as a request, message, uploaded file or a record users can edit.",
            "none": "It requests no URL.",
        },
    })
}

/// The option of the destination Choice that keeps error details a concern.
pub const CLIENT: &str = "client";

/// Where a function's text goes, asked when an error-detail signal stays
/// undecided. An error or body shaped for a response counts as the client:
/// helpers that format errors for a server's callers return them.
pub fn security_destination(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("Where does the text that `{code}` produces or passes on go?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "client": "Into a response to a request from another computer: an HTTP, API or RPC response, a message to a connected client, or an error, status or body shaped for such a response that it builds or returns.",
            "local": "To the person running a local program: a terminal, console, window, or a report or file on their own machine.",
            "logs": "To logs, or to the program's own error reporting or monitoring.",
            "caller": "Back to the code that called it as an ordinary error or value, such as a parse, lookup or validation failure, not shaped as a response.",
            "stored": "Into a database, queue, cache or job record.",
        },
    })
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

/// Which created error's message carries another error's text. Asking per
/// message decided error details the response path left open: on MyOasis
/// units whose handler is in another file, 21 of 25 chose `none` decisively
/// while real leaks (`error.message` in the message) were picked.
pub fn security_message_origin(ids: &[String]) -> Value {
    choose_id(
        "Which entry in `messages` puts the text of an error the program did not create, such as a database or library error message, into the message?",
        format!(
            "Options are the `id` values in `messages`, each the message argument of an error the function creates. {EVIDENCE}"
        ),
        ids,
        "Every message is text the program writes itself; an error may only be attached as a cause.",
    )
}

/// Whether the error handler a web framework calls for every thrown error
/// sends clients more than the program's own messages and codes. One
/// question per handler in place of an undecided answer in each function
/// whose response it writes: on labeled handlers a safe one scored 0.19 and
/// leaking ones 0.96.
pub fn security_handler_leaks() -> Value {
    json!({
        "type": "noul",
        "instructions": {
            "question": "Does the error handler in `error_handler.source` send a remote client anything besides the message and code of errors the program raises itself?",
            "note": format!("`error_classes` defines the program's own errors, when found; `error_handler.helpers` holds functions of its file that it calls. {EVIDENCE}"),
        },
        "criteria": {
            "true": "It sends an error's cause, its stack trace, or the message of an error the program did not create, such as a database or library error.",
            "false": "It sends only the message and code of the program's own errors, and a fixed message for any other error; causes and stacks go to logs.",
        },
    })
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
    pub(super) question: &'static str,
    pub(super) yes: &'static str,
    pub(super) no: &'static str,
    /// Examples of the "false" answer, when the plain criterion is not enough.
    pub(super) no_examples: &'static [&'static str],
}

impl Check {
    pub fn body(&self, code: &str) -> Value {
        let mut body = noul(self.question.replace("{code}", code), self.yes, self.no);
        if !self.no_examples.is_empty() {
            body["criteria"]["false"] = json!({"what": self.no, "examples": self.no_examples});
        }
        body
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
/// markup check names text shown as a JSX child and CSS values as escaped or
/// inert: without them, template strings in JSX left most React units
/// undecided. The
/// path check names the program's own directories as safe: without them, a
/// third of injection units stayed undecided on paths the program builds from
/// its project root. One
/// broad question ("is every variable bound, escaped or checked?") stayed
/// undecided even for `eval` of model output; one literal check per kind
/// decides, and names the kind.
pub const UNHANDLED: [Check; 7] = [
    Check {
        id: "sql",
        question: "Does `{code}` put a variable into the text of an SQL query instead of passing it as a bound parameter?",
        yes: "A variable is joined, formatted or interpolated into SQL text that is then run, including text passed to `$queryRawUnsafe`, `$executeRawUnsafe`, `Prisma.raw` or `sql.raw`.",
        no: "Values are passed as bound parameters or placeholders; identifiers such as table and column names come from a fixed list or the database schema, or are quoted by a function that wraps them in double quotes and doubles any double quote inside; or it runs no SQL.",
        no_examples: &[
            "A value placed with ${…} in a tagged template that binds it as a parameter, such as sql`SELECT * FROM posts WHERE id = ${id}` in Drizzle, postgres.js or Vercel Postgres, or Prisma's $queryRaw`…${id}…` and Prisma.sql`…${id}…`",
        ],
    },
    Check {
        id: "shell",
        question: "Does `{code}` run a shell command string that holds a variable?",
        yes: "A command line built with a variable is run through a shell, such as with exec, execSync, os.system, subprocess with shell=True, or sh -c.",
        no: "It runs programs with a list of arguments and no shell, quotes each variable for the shell, or runs no command.",
        no_examples: &[],
    },
    Check {
        id: "code",
        question: "Does `{code}` evaluate text that holds a variable as code or as a template?",
        yes: "It passes text that holds a variable to eval, exec, new Function, a template compiler or a similar evaluator.",
        no: "It parses data with a data-only parser such as JSON or a literal parser, evaluates only fixed code, or builds a database query or markup, which is not code it evaluates.",
        no_examples: &[],
    },
    Check {
        id: "markup",
        question: "Does `{code}` put a variable into HTML or SVG markup without escaping it?",
        yes: "A variable is joined into HTML or SVG text, or assigned to innerHTML, React's dangerouslySetInnerHTML or a similar raw-markup property, without an escaping or sanitizing function.",
        no: "Values go through an escaping function or a template or component that escapes them, or it builds no markup.",
        no_examples: &[
            "Text shown as a JSX child, such as {`Total: ${count}`} inside an element, which React escapes",
            "A CSS value or class name built from a variable, such as a `style` property or `className`",
        ],
    },
    Check {
        id: "path",
        question: "Does `{code}` open, write or delete a file at a path built from a variable without checking that it stays inside a directory?",
        yes: "A path is built from a variable that can hold a name or path from outside the program, such as a request, upload, archive entry or user input, and is used without reducing it to a base name, rejecting parent-directory parts, or checking that the resolved path stays under a base directory.",
        no: "Such paths are checked; are built from the program's own directories, such as its project root, data or cache directory, joined with names the program chooses; come from the program's configuration or the command line of the person running it; or it uses no such path.",
        no_examples: &[],
    },
    Check {
        id: "url",
        question: "Does `{code}` request a URL or host taken from a variable without checking the host?",
        yes: "It sends a request to a URL or host that comes from a variable, without checking the host against an allowed list or rejecting private addresses.",
        no: "The host is fixed, comes from the program's configuration, or is checked; the request is sent by code running in the user's browser, such as a web page script or a client component, which reaches only what that user can; the URL is only where it redirects the client; or it requests no URL.",
        no_examples: &[],
    },
    Check {
        id: "redirect",
        question: "Does `{code}` redirect the client to a URL or path taken from a variable without checking where it leads?",
        yes: "A URL or path that can come from a request, form field, query parameter or stored user input is passed to a redirect, such as redirect(), NextResponse.redirect, res.redirect or a Location header, without checking that it is a path on the program's own site or that its host is on an allowed list.",
        no: "The target is fixed, is the program's own origin joined with a fixed path, is checked to be a path on its own site (a single leading slash) or a host on an allowed list, comes from the program's configuration, or it does not redirect.",
        no_examples: &[],
    },
];

/// Specific weak settings, asked when the broad presence question is not clear.
pub const WEAK_SETTINGS: [Check; 6] = [
    Check {
        id: "tls",
        question: "Does `{code}` turn off certificate or host name verification?",
        yes: "It turns off certificate or host name checks, or accepts invalid certificates or host names.",
        no: "It keeps verification on, or makes no TLS connection.",
        no_examples: &[],
    },
    Check {
        id: "hash",
        question: "Does `{code}` hash passwords or derive keys from them with a fast or broken hash, or with few iterations?",
        yes: "It hashes passwords or derives keys from them with MD5, SHA-1, a single round of SHA-256, or a key derivation function with few iterations.",
        no: "It uses bcrypt, scrypt, Argon2 or a key derivation function with many iterations, or it does not handle passwords.",
        no_examples: &[],
    },
    Check {
        id: "random",
        question: "Does `{code}` make a token, code, password or identifier that must be unguessable with a non-cryptographic random generator?",
        yes: "It uses a generator such as Math.random or Python's random module for a secret value, such as a session token, reset or verification code, or random password.",
        no: "It uses a cryptographic generator such as crypto.randomUUID, secrets or OsRng, or the random value is not a secret.",
        no_examples: &[],
    },
    Check {
        id: "cors",
        question: "Does `{code}` let pages from origins it does not fully check read its responses with credentials?",
        yes: "It allows any origin, reflects the request's origin, or matches origins loosely, such as by suffix or substring, while allowing credentials.",
        no: "It allows only listed origins by exact match, allows no credentials, or sets no cross-origin rules.",
        no_examples: &[],
    },
    Check {
        id: "cookie",
        question: "Does `{code}` set or configure a session or authentication cookie without the Secure or HttpOnly flag?",
        yes: "A cookie that holds a session or token is set or configured without Secure or without HttpOnly.",
        no: "Such cookies have both flags, the cookie holds no session or token, or the code sets no cookie.",
        no_examples: &[],
    },
    Check {
        id: "public_secret",
        question: "Does `{code}` read a secret from an environment variable that the build puts into browser code?",
        yes: "A secret API key, service-role key, signing or webhook secret, database URL or password is read from a variable whose prefix makes the build inline it into browser code, such as NEXT_PUBLIC_, VITE_, REACT_APP_, PUBLIC_ or EXPO_PUBLIC_, or is listed under `env` in a Next.js configuration.",
        no: "Such variables hold only values meant for browsers, such as publishable or anonymous keys, public URLs and site ids; secrets come from variables without such a prefix; or it reads no such variable.",
        no_examples: &[],
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
        no_examples: &[],
    },
    Check {
        id: "exception_to_client",
        question: "Does `{code}` send an exception's message, stack trace or a database error to a remote client in a response?",
        yes: "The text of an exception it did not raise itself to explain bad input, or a stack trace, is put into the response to a request.",
        no: "Responses carry fixed messages or codes, or only messages the program wrote to explain invalid input; details stay in server logs.",
        no_examples: &[],
    },
];
