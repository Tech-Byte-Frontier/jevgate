//! Questions about security in application code: whether a function places
//! variables into interpreted text, logs secrets or weakens a setting, and the
//! literal checks per kind that the trace and caller rechecks ask.
use super::{EVIDENCE, choose_id, deserializers::DESERIALIZE, noul, score};
use serde_json::{Value, json};

/// Whether a function places a variable into text another program runs or
/// renders. Presence only: the trace questions decide whether it is a concern.
/// In Django code it also names markup marked safe and deserializers, since
/// `mark_safe`, `|safe` templates and `pickle.loads` of request data were
/// the injections its views held. Other code whose source names a
/// deserializer that can build any object is asked about it too, naming
/// `deserializers` (see [`super::deserializers_named`]), and code that
/// names an XML parser able to resolve external entities (`xml`, see
/// [`super::parses_xml`]) about parsing with it.
pub fn security_interpreted(
    code: &str,
    django: bool,
    rendered: bool,
    deserializers: Option<&str>,
    xml: bool,
) -> Value {
    let mut question = format!(
        "Does `{code}` place a variable into the text of a database query, shell command, code to evaluate, or HTML markup"
    );
    let mut yes = String::from(
        "A variable is joined, formatted or interpolated into the text of a query, command, code or markup that is then run or rendered",
    );
    let mut no = String::from(
        "Variables are passed only as bound parameters, separate arguments, or through a template or component that escapes them; the text is built only from fixed values;",
    );
    if django {
        question.push_str(", or load it with a deserializer that can build any object");
        yes.push_str(", marked as safe markup or passed to a template that writes it unescaped, or loaded with pickle or a similar deserializer");
        no.push_str(" data is parsed only as JSON or another data-only format;");
    } else if let Some(names) = deserializers {
        if rendered {
            yes.push_str(", or passed to a template that writes it unescaped");
        }
        question.push_str(", or load it with a deserializer that can build any object");
        yes.push_str(&format!(
            ", or loaded with a deserializer that can build any object or run code, such as {names}"
        ));
        no.push_str(" data is parsed only as JSON or another data-only format;");
    } else if rendered {
        yes.push_str(", or passed to a template that writes it unescaped");
    }
    if xml {
        // Both clauses would make the question too long to read as one.
        question = if django || deserializers.is_some() {
            format!(
                "Does `{code}` pass a variable into query, command, code or markup text, a deserializer that can build any object, or an XML parser that resolves external entities"
            )
        } else {
            question + ", or parse it as XML with a parser that resolves external entities"
        };
        yes.push_str(
            ", or parsed as XML with external entities or document type definitions enabled",
        );
        no.push_str(
            " XML is parsed with document type definitions and external entities turned off;",
        );
    }
    question.push('?');
    yes.push('.');
    no.push_str(" or the function builds no such text.");
    noul(question, &yes, &no)
}

/// In Django code, redirects count too: a view that redirects to a URL from
/// the request is an open redirect. Redirects to the program's own paths
/// with an id in them are named as fine, since nearly every view has one.
pub fn security_resource(code: &str, django: bool) -> Value {
    if django {
        return noul(
            format!(
                "Does `{code}` open, write or request a file path or URL, or redirect the client to a URL or host, that is taken or built from a variable?"
            ),
            "The path of a file it opens, writes or deletes, the URL it requests, or the whole URL or host it redirects the client to, comes from or is built with a variable such as a parameter or a value read from input.",
            "Every path and URL it uses is fixed in the code or read from the program's configuration; it redirects only to route names or the program's own paths, even with an id or name from a variable in them; or it uses none.",
        );
    }
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
/// client" alone matched a front end posting a stack to its own server. In
/// Django code, exceptions a view lets propagate reach the framework's error
/// handling, which is judged apart, and validation errors carry messages
/// written for the user.
pub fn security_error_details(code: &str, django: bool) -> Value {
    let no = if django {
        "It returns generic messages or error codes, or the messages of validation errors such as Django's ValidationError, which are written for the user; lets exceptions propagate to the framework's error handling; keeps details in server logs or sends them to its own error-reporting service; or sends no response."
    } else {
        "It returns generic messages or error codes, keeps details in server logs or sends them to its own error-reporting service, shows errors to the local user of a command-line or desktop program, or sends no response."
    };
    noul(
        format!(
            "Does `{code}` put internal error details, such as stack traces, database errors or server paths, into the response it sends to a remote client?"
        ),
        "It puts an exception's stack trace, a database or library error message, a query, or a server path into the response to a request from a remote client.",
        no,
    )
}

/// In Django code the examples add the settings a settings module decides:
/// debug mode, CSRF protection, and a secret key written in the code.
pub fn security_weakened(code: &str, django: bool) -> Value {
    let mut yes = vec![
        "Certificate or signature verification turned off",
        "Passwords hashed with a fast or broken hash such as MD5 or SHA-1",
        "Tokens, passwords or identifiers that must be unguessable made with a non-cryptographic random generator",
        "Any origin allowed to send credentialed requests",
        "Session cookies set without HttpOnly or Secure",
        "A secret key read from an environment variable whose prefix, such as NEXT_PUBLIC_, makes the build put it into browser code",
    ];
    let mut no = vec![
        "Checksums, cache keys or content hashes",
        "Shuffling, sampling or visual effects",
        "Test data or local development tools",
    ];
    if django {
        yes.extend([
            "Debug mode turned on, or cross-site request forgery protection turned off, for the deployed site",
            "A secret key or password written as a literal in the code, even when shown redacted",
        ]);
        no.push(
            "Shared settings whose weak values a settings module for production that imports them sets again",
        );
    }
    json!({
        "type": "noul",
        "instructions": {
            "question": format!("Does `{code}` turn off a security check or choose a weak security setting?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "true": {
                "what": "It weakens protection that the program relies on.",
                "examples": yes,
            },
            "false": {
                "what": "It uses secure defaults or strong settings, or uses such functions where security does not depend on them.",
                "examples": no,
            }
        },
    })
}

/// Where the variables of a query, command, markup, path or URL come from.
/// The middle level (parameters of unknown origin) is a concern for a
/// caller to settle, not a sign that the code is fine.
/// In Django code the program's own values name a management command's
/// options and the files the program ships, which its commands read.
pub fn security_origin(code: &str, callers: bool, django: bool) -> Value {
    let own = if django {
        "Only from the program itself: constants, fixed choices, numbers, values checked against an allowed list, the deployment's configuration, files the program ships in its own directories, or the command line and settings of the person running a local program or a management command."
    } else {
        "Only from the program itself: constants, fixed choices, numbers, values checked against an allowed list, the deployment's configuration, or the command line and settings of the person running a local program."
    };
    score(
        format!(
            "Where do the variables that `{code}` places into a query, command, code, markup, file path or URL come from?"
        ),
        if callers { CALLERS } else { "" },
        [
            own,
            "From the function's parameters or other data whose origin this code does not show.",
            "From another party: a network request, message, uploaded file, or a record other users can edit, used as received, including the arguments of a function that clients call directly as an endpoint.",
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

/// Which created error's message carries another error's text. Asking per
/// message decided error details the response path left open: on MyOasis
/// units whose handler is in another file, 21 of 25 chose `none` decisively
/// while real leaks (`error.message` in the message) were picked. With the
/// errors its callees create in view (`callees`), passing their text on is
/// the program's own: a FastAPI route that returns the `LookupError` its
/// service raises with its own not-found text was picked at 0.93.
pub fn security_message_origin(ids: &[String], callees: bool) -> Value {
    let (callee_note, none) = if callees {
        (
            " `errors_created_by_functions_it_calls` lists errors the functions it calls raise with the program's own text: passing on the text of an error caught there is the program's own when only those errors can be caught.",
            "Every message is text the program writes itself, or passes on the text of errors from `errors_created_by_functions_it_calls`; an error may only be attached as a cause.",
        )
    } else {
        (
            "",
            "Every message is text the program writes itself; an error may only be attached as a cause.",
        )
    };
    choose_id(
        "Which entry in `messages` puts the text of an error the program did not create, such as a database or library error message, into the message?",
        format!(
            "Options are the `id` values in `messages`, each the message argument of an error the function creates.{callee_note} {EVIDENCE}"
        ),
        ids,
        none,
    )
}

/// Whether the error handler a web framework calls for every thrown error
/// sends clients more than the program's own messages and codes. One
/// question per handler in place of an undecided answer in each function
/// whose response it writes: on labeled handlers a safe one scored 0.19 and
/// leaking ones 0.96. A Django handler may pass on the framework's own API
/// errors, whose messages are written for the user, and leave other errors
/// to the framework's generic error page.
pub fn security_handler_leaks(django: bool) -> Value {
    let (question, no) = if django {
        (
            "Does the error handler in `error_handler.source` send a remote client anything besides the message and code of errors the program raises itself and of the framework's errors written for the user?",
            "It sends only the message and code of the program's own errors and of the framework's errors written for the user, such as Django REST framework's validation, not-found and permission errors, and a fixed message for any other error or none, leaving it to the framework's generic error page; causes and stacks go to logs. Django REST framework's own exception_handler answers only its API errors and returns None for any other error.",
        )
    } else {
        (
            "Does the error handler in `error_handler.source` send a remote client anything besides the message and code of errors the program raises itself?",
            "It sends only the message and code of the program's own errors, and a fixed message for any other error; causes and stacks go to logs.",
        )
    };
    json!({
        "type": "noul",
        "instructions": {
            "question": question,
            "note": format!("`error_classes` defines the program's own errors, when found; `error_handler.helpers` holds functions of its file that it calls. {EVIDENCE}"),
        },
        "criteria": {
            "true": "It sends an error's cause, its stack trace, or the message of an error the program did not create, such as a database or library error.",
            "false": no,
        },
    })
}

/// In Django code, settings modules count: a `dev.py` or `test.py` exists to
/// relax the deployed settings.
pub fn security_dev_only(code: &str, django: bool) -> Value {
    if django {
        return noul(
            format!(
                "Does `{code}` run only in tests, local development tools or scripts a developer runs by hand?"
            ),
            "It is test code, a local development tool, a script for the developer, or settings that only tests and local development run with, not part of what is deployed or shipped to users.",
            "It is part of the application that is deployed or shipped to users, including settings a deployed server runs with.",
        );
    }
    noul(
        format!(
            "Does `{code}` run only in tests, local development tools or scripts a developer runs by hand?"
        ),
        "It is test code, a local development tool, or a script for the developer, not part of what is deployed or shipped to users.",
        "It is part of the application that is deployed or shipped to users.",
    )
}

/// Which site holds what a presence question found, to locate the finding.
pub fn security_site(what: &str, sites: &[String], code: &str) -> Value {
    choose_id(
        &format!("Which entry in `sites` {what}?"),
        format!("Options are the `id` values in `sites`, each a statement of `{code}`. {EVIDENCE}"),
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

pub(super) const CALLERS: &str = "`callers` holds functions that call it.";

/// Whether a variable reaches each kind of interpreted text unhandled. The
/// markup check names text shown as a JSX child and CSS values as escaped or
/// inert: without them, template strings in JSX left most React units
/// undecided. The
/// path check names the program's own directories as safe: without them, a
/// third of injection units stayed undecided on paths the program builds from
/// its project root. One
/// broad question ("is every variable bound, escaped or checked?") stayed
/// undecided even for `eval` of model output; one literal check per kind
/// decides, and names the kind. The redirect check names a destination a
/// user saved on purpose as handled: a link shortener's redirect to the
/// target its owner saved was a review.
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
        yes: "A URL or path that a request carries, such as a query parameter, form field, header or cookie, or an argument of a function that clients call directly, is passed to a redirect, such as redirect(), NextResponse.redirect, res.redirect, router.push or a Location header, without checking that it is a path on the program's own site or that its host is on an allowed list.",
        no: "The target is fixed, is the program's own origin joined with a fixed path, is checked to be a path on its own site (a single leading slash) or a host on an allowed list, comes from the program's configuration, or is a destination a signed-in user saved on purpose, such as the target of a short link; or it does not redirect.",
        no_examples: &[],
    },
];

const DJANGO_PATH: Check = Check {
    id: "path",
    question: "Does `{code}` open, write or delete a file at a path built from a variable without checking that it stays inside a directory?",
    yes: "A path is built from a variable that can hold a name or path from outside the program, such as a request, upload, archive entry or user input, and is used without reducing it to a base name, rejecting parent-directory parts, or checking that the resolved path stays under a base directory.",
    no: "Such paths are checked; are built from the program's own directories, such as its project root, data or cache directory, joined with names the program chooses; come from the program's configuration or the command line of the person running it; or it uses no such path.",
    no_examples: &[
        "A file saved through Django's storage API, such as a file field's save or default_storage.save, which keeps names inside the storage's root",
        "Files listed from one of the program's own directories, such as its fixtures",
    ],
};

const DJANGO_SQL: Check = Check {
    id: "sql",
    question: "Does `{code}` put a variable into the text of an SQL query instead of passing it as a bound parameter?",
    yes: "A variable is joined, formatted or interpolated into SQL text that is then run, such as with %, + or an f-string passed to execute, raw, extra or RawSQL.",
    no: "Values are passed as bound parameters, placeholders or the params argument, or through the ORM's filters; identifiers such as table and column names come from a fixed list or the database schema, or are quoted by a function that wraps them in double quotes and doubles any double quote inside; or it runs no SQL.",
    no_examples: &[],
};

const DJANGO_MARKUP: Check = Check {
    id: "markup",
    question: "Does `{code}` put a variable into HTML or SVG markup without escaping it, itself or through a template it renders?",
    yes: "A variable is joined into HTML or SVG text, marked as safe markup with mark_safe or SafeString, or passed to a template that writes it with a safe filter or with autoescaping off, without an escaping function.",
    no: "Values go through an escaping function or a template that escapes them, or it builds no markup.",
    no_examples: &[
        "A template rendered with the variable in its context, when the template writes that value without a safe filter, which Django escapes",
        "format_html or format_html_join with the variables passed as its arguments, which escapes them",
    ],
};

/// The markup check of a function outside Django that renders a template
/// writing values without escaping: DVNA's product search hands the
/// request's search term to `views/app/products.ejs`, which writes it with
/// `<%- … %>`, and the function alone read as building no markup.
pub const VIEW_MARKUP: Check = Check {
    id: "markup",
    question: "Does `{code}` put a variable into HTML or SVG markup without escaping it, itself or through a template it renders?",
    yes: "A variable is joined into HTML or SVG text, or passed to a template that writes it without escaping, such as with EJS `<%- … %>`, Handlebars `{{{ … }}}` or a `|safe` filter, without an escaping function.",
    no: "Values go through an escaping function or a template that escapes them, or it builds no markup.",
    no_examples: &[
        "A template rendered with the variable, when the template writes that value with an escaping tag, such as EJS `<%= … %>` or Handlebars `{{ … }}`",
    ],
};

/// Specific weak settings, asked when the broad presence question is not clear.
/// Turning off output escaping had no check: NodeGoat's `autoescape: false`
/// and RailsGoat's `escape_html_entities_in_json = false` were at most notes.
pub const WEAK_SETTINGS: [Check; 7] = [
    Check {
        id: "tls",
        question: "Does `{code}` turn off certificate or host name verification?",
        yes: "It turns off certificate or host name checks, or accepts invalid certificates or host names.",
        no: "It keeps verification on, or makes no TLS connection.",
        no_examples: &[],
    },
    Check {
        id: "hash",
        question: "Does `{code}` keep passwords as plain text, or hash them or derive keys from them with a fast or broken hash, or with few iterations?",
        yes: "It saves passwords, or checks a login against saved passwords, as plain text, or hashes passwords or derives keys from them with MD5, SHA-1, a single round of SHA-256, or a key derivation function with few iterations.",
        no: "It uses bcrypt, scrypt, Argon2 or a key derivation function with many iterations; it hands passwords to a library, framework or model hook that hashes them before saving; or it does not handle passwords.",
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
        no_examples: &[
            "A cookie added to a request, such as Go's `r.AddCookie`, rather than set on a response",
            "A cookie set empty and already expired, which deletes it",
        ],
    },
    Check {
        id: "public_secret",
        question: "Does `{code}` read a secret from an environment variable that the build puts into browser code?",
        yes: "A secret API key, service-role key, signing or webhook secret, database URL or password is read from a variable whose prefix makes the build inline it into browser code, such as NEXT_PUBLIC_, VITE_, REACT_APP_, PUBLIC_ or EXPO_PUBLIC_, or is listed under `env` in a Next.js configuration.",
        no: "Such variables hold only values meant for browsers, such as publishable or anonymous keys, public URLs and site ids; secrets come from variables without such a prefix; or it reads no such variable.",
        no_examples: &[],
    },
    Check {
        id: "escape",
        question: "Does `{code}` turn off the automatic escaping of values written into HTML?",
        yes: "It turns off a template engine's or serializer's escaping of HTML for output that browsers render, such as autoescape set to false or escape_html_entities_in_json set to false.",
        no: "Escaping stays on; the output is not HTML that browsers render, such as Markdown, plain-text email or source code; or it configures no escaping.",
        no_examples: &[],
    },
];

/// The token and key checks of code outside C# and Django, which ask their
/// own: a JWT decoded without verifying its signature found the broad
/// question at 0.93 to 0.98 in DVGA and JavaVulnerableLab, and with no check
/// to name the setting it was only a note; DVNA's session secret
/// `'keyboard cat'` and RailsGoat's encryption key were missed.
pub const TOKEN_AND_KEY: [Check; 2] = [TOKEN, KEY];

const KEY: Check = Check {
    id: "key",
    question: "Does `{code}` sign or encrypt with a key or secret written in the code?",
    yes: "A signing or encryption key, such as the secret that signs session cookies or JSON Web Tokens, or a key that encrypts stored data, is a string or bytes written in the code or a constant of the program.",
    no: "Keys are read from configuration, the environment or a secret store; the literal is only a placeholder, or is used only in tests or local development; or it uses no key.",
    no_examples: &[],
};

const TOKEN: Check = Check {
    id: "token",
    question: "Does `{code}` accept security tokens without verifying their signature or expiry?",
    yes: "It trusts a JSON Web Token or other signed token without verifying its signature, such as a decode call with signature verification turned off, a decode used where a verify is needed, or an algorithm list that allows none, or it turns off the expiry check.",
    no: "Signatures and expiry are checked where tokens are accepted, or it accepts none: it only creates, stores or sends a token, reads the claims of a token already verified before it, or checks that one is present while a server verifies it.",
    no_examples: &[],
};

const DJANGO_HASH: Check = Check {
    id: "hash",
    question: "Does `{code}` hash passwords or derive keys from them with a fast or broken hash, or with few iterations?",
    yes: "It hashes passwords or derives keys from them with MD5, SHA-1, a single round of SHA-256, or a key derivation function with few iterations, or lists such a hasher first in PASSWORD_HASHERS.",
    no: "It uses bcrypt, scrypt, Argon2 or a key derivation function with many iterations; it hashes through Django's set_password, make_password or a form's save, whose hasher the settings choose; or it does not handle passwords.",
    no_examples: &[],
};

const DJANGO_TLS: Check = Check {
    id: "tls",
    question: "Does `{code}` turn off certificate, host name or signature verification?",
    yes: "It turns off certificate or host name checks, accepts invalid certificates or host names, or decodes a signed token such as a JWT without verifying its signature.",
    no: "It keeps verification on, or makes no TLS connection and reads no signed token.",
    no_examples: &[],
};

const DJANGO_CORS: Check = Check {
    id: "cors",
    question: "Does `{code}` set cross-origin rules that let pages from origins it does not fully check read the deployed site's responses with credentials?",
    yes: "It allows any origin, reflects the request's origin, or matches origins loosely, such as by suffix or substring, while allowing credentials, in code or settings that the deployed site uses.",
    no: concat!(
        "It allows only listed origins by exact match or allows no credentials; it sets no cross-origin rules itself, ",
        "whatever the site's settings choose; or a settings module for production that imports these settings sets it again."
    ),
    no_examples: &[],
};

const DJANGO_COOKIE: Check = Check {
    id: "cookie",
    question: "Does `{code}` set or configure a session or authentication cookie of the deployed site without the Secure or HttpOnly flag?",
    yes: "A cookie that holds a session or token is set or configured without Secure or without HttpOnly, in code or settings that the deployed site uses.",
    no: concat!(
        "Such cookies have both flags, the cookie holds no session or token, or the code sets no cookie; ",
        "or a settings module for production that imports these settings sets it again."
    ),
    no_examples: &[],
};

const DEBUG: Check = Check {
    id: "debug",
    question: "Does `{code}` turn on a web framework's debug mode or detailed error pages for the deployed site?",
    yes: "It turns debug mode on, such as DEBUG = True, in code or settings that the deployed site uses.",
    no: "Debug mode is off, is read from the environment with off as the default, is turned on only in settings for tests or local development, or a settings module for production that imports these settings sets it again.",
    no_examples: &[],
};

const CSRF: Check = Check {
    id: "csrf",
    question: "Does `{code}` turn off protection against cross-site request forgery for requests that change data?",
    yes: "A view or route that changes data as the user its session cookie signs in, such as their profile, password or records, is exempted from the CSRF check, such as with csrf_exempt, or the CSRF middleware or check is removed.",
    no: "CSRF protection stays on; the exempted endpoint authenticates each request itself rather than with the session cookie, such as a webhook that verifies a signature, an API that reads a token from a header, or a form for visitors who are not signed in that asks for a password reset email or checks a reset token it is sent; it only reads data; or it sets nothing about CSRF.",
    no_examples: &[],
};

const SECRET: Check = Check {
    id: "literal_secret",
    question: "Does `{code}` set a signing key, password or token that the deployed site uses to a literal written in the code?",
    yes: "A secret key, password, token or API key that the deployed program uses is a literal in the code, including one shown as a redacted literal.",
    no: "Secrets are read from the environment, a file or a secret store; the literal is empty or only a placeholder; it is used only in tests or local development; or a settings module for production that imports these settings sets it again.",
    no_examples: &[],
};

/// Specific exposures, asked when the broad presence questions are not clear:
/// a logged configuration or argument list that holds a password, and an
/// exception's own text in a response, were left undecided by them.
pub const EXPOSURES: [Check; 2] = [LOGS_OBJECT_SECRET, EXCEPTION_TO_CLIENT];

const LOGS_OBJECT_SECRET: Check = Check {
    id: "logs_object_secret",
    question: "Does `{code}` log an object, configuration, request, command or list of arguments that holds a password, token or key?",
    yes: "It logs a whole object, configuration, request, command line or argument list, and that value holds a password, token, key or secret.",
    no: "It logs only values that hold no secret, masks secrets before logging, or logs nothing.",
    no_examples: &[],
};

const EXCEPTION_TO_CLIENT: Check = Check {
    id: "exception_to_client",
    question: "Does `{code}` send an exception's message, stack trace or a database error to a remote client in a response?",
    yes: "The text of an exception it did not raise itself to explain bad input, or a stack trace, is put into the response to a request.",
    no: "Responses carry fixed messages or codes, or only messages the program wrote to explain invalid input; details stay in server logs.",
    no_examples: &[],
};

/// The exception check of a unit sent with the errors the functions it
/// calls create: "an exception it did not raise itself" held for a handler
/// returning the message its service raised to explain a missing record,
/// which stayed a review with the service's raise in view. It asks whether
/// the text describes an internal failure, not who raised it: asked about
/// a library's exceptions, a benchmark returning a framework's bad-request
/// message (`MultiPartException`) became a review.
pub const EXCEPTION_TO_CLIENT_FROM_CALLEES: Check = Check {
    id: "exception_to_client",
    question: EXCEPTION_TO_CLIENT.question,
    yes: "The text of an exception that describes an internal failure, such as a database, network, file system or library error, or a stack trace, is put into the response to a request, such as the message of every exception it catches.",
    no: "Responses carry fixed messages or codes, or messages written to explain invalid input or a missing record to the client, whether the program's own, such as the errors in `errors_created_by_functions_it_calls`, or a framework's validation and bad-request errors; details stay in server logs.",
    no_examples: &[],
};

const DJANGO_EXCEPTION_TO_CLIENT: Check = Check {
    id: "exception_to_client",
    question: "Does `{code}` send an exception's message, stack trace or a database error to a remote client in a response?",
    yes: "The text of an exception it did not raise itself to explain bad input, or a stack trace, is put into the response to a request.",
    no: "Responses carry fixed messages or codes, or only messages written to explain invalid input, such as those of Django's or a form's ValidationError; exceptions it does not catch go to the framework's error handling; details stay in server logs.",
    no_examples: &[],
};

const ENVIRONMENT_TO_CLIENT: Check = Check {
    id: "environment_to_client",
    question: "Does `{code}` send the server's environment variables, settings or whole request metadata to a remote client?",
    yes: "It puts the process environment, the application's settings, or a whole request metadata object such as Django's request.META, which holds the server's environment, into a response or a page it renders.",
    no: "It sends only chosen fields meant for the client, such as the user's own name or a public setting, or sends no such data.",
    no_examples: &[],
};

const DJANGO_REDIRECT: Check = Check {
    id: "redirect",
    question: "Does `{code}` redirect the client to a URL or path taken from a variable without checking where it leads?",
    yes: "A URL or path that a request carries, such as a query parameter, form field, header or cookie, is passed to redirect(), HttpResponseRedirect or a Location header without checking that it is a path on the program's own site or that its host is on an allowed list.",
    no: "The target is fixed, is a route name or built with reverse(), is one of the program's own paths with only ids or names from variables in it, is checked such as with url_has_allowed_host_and_scheme, comes from the program's configuration, or is read from a stored record rather than the request; or it does not redirect.",
    no_examples: &[],
};

/// Checks asked of Django code in place of the common check with the same
/// id: they name Django's raw queries (`raw`, `extra`, `RawSQL`), safe
/// markup and templates, storage API, redirects, password hashers and
/// validation errors, and ask cookie, CORS and verification settings about
/// the deployed site, since settings modules that production imports and
/// overrides were flagged when asked about the module alone.
pub const DJANGO_VARIANTS: [Check; 9] = [
    DJANGO_SQL,
    DJANGO_MARKUP,
    DJANGO_PATH,
    DJANGO_REDIRECT,
    DJANGO_TLS,
    DJANGO_HASH,
    DJANGO_CORS,
    DJANGO_COOKIE,
    DJANGO_EXCEPTION_TO_CLIENT,
];

/// The injection check Django code adds: request data given to a
/// deserializer that can build any object (`pickle.loads(request.body)`).
pub const DJANGO_UNHANDLED: [Check; 1] = [DESERIALIZE];

/// The weak settings Django code adds, which its settings modules and view
/// decorators decide: debug mode, CSRF protection and a literal secret key.
pub const DJANGO_SETTINGS: [Check; 3] = [DEBUG, CSRF, SECRET];

/// The exposure Django code adds: `request.META` or the settings, which
/// hold the server's environment, sent to a client.
pub const DJANGO_EXPOSURES: [Check; 1] = [ENVIRONMENT_TO_CLIENT];
