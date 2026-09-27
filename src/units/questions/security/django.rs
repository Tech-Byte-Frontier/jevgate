//! The checks Django code is asked in its own words: its ORM, templates and
//! settings, and the decorators and middleware that decide them.
use super::*;

pub(super) const DJANGO_PATH: Check = Check {
    id: "path",
    question: "Does `{code}` open, write or delete a file at a path built from a variable without checking that it stays inside a directory?",
    yes: "A path is built from a variable that can hold a name or path from outside the program, such as a request, upload, archive entry or user input, and is used without reducing it to a base name, rejecting parent-directory parts, or checking that the resolved path stays under a base directory.",
    no: "Such paths are checked; are built from the program's own directories, such as its project root, data or cache directory, joined with names the program chooses; come from the program's configuration or the command line of the person running it; or it uses no such path.",
    no_examples: &[
        "A file saved through Django's storage API, such as a file field's save or default_storage.save, which keeps names inside the storage's root",
        "Files listed from one of the program's own directories, such as its fixtures",
    ],
};

pub(super) const DJANGO_SQL: Check = Check {
    id: "sql",
    question: "Does `{code}` put a variable into the text of an SQL query instead of passing it as a bound parameter?",
    yes: "A variable is joined, formatted or interpolated into SQL text that is then run, such as with %, + or an f-string passed to execute, raw, extra or RawSQL.",
    no: "Values are passed as bound parameters, placeholders or the params argument, or through the ORM's filters; identifiers such as table and column names come from a fixed list or the database schema, or are quoted by a function that wraps them in double quotes and doubles any double quote inside; or it runs no SQL.",
    no_examples: &[],
};

pub(super) const DJANGO_MARKUP: Check = Check {
    id: "markup",
    question: "Does `{code}` put a variable into HTML or SVG markup without escaping it, itself or through a template it renders?",
    yes: "A variable is joined into HTML or SVG text, marked as safe markup with mark_safe or SafeString, or passed to a template that writes it with a safe filter or with autoescaping off, without an escaping function.",
    no: "Values go through an escaping function or a template that escapes them, or it builds no markup.",
    no_examples: &[
        "A template rendered with the variable in its context, when the template writes that value without a safe filter, which Django escapes",
        "format_html or format_html_join with the variables passed as its arguments, which escapes them",
    ],
};

pub(super) const DJANGO_HASH: Check = Check {
    id: "hash",
    question: "Does `{code}` hash passwords or derive keys from them with a fast or broken hash, or with few iterations?",
    yes: "It hashes passwords or derives keys from them with MD5, SHA-1, a single round of SHA-256, or a key derivation function with few iterations, or lists such a hasher first in PASSWORD_HASHERS.",
    no: "It uses bcrypt, scrypt, Argon2 or a key derivation function with many iterations; it hashes through Django's set_password, make_password or a form's save, whose hasher the settings choose; or it does not handle passwords.",
    no_examples: &[],
};

pub(super) const DJANGO_TLS: Check = Check {
    id: "tls",
    question: "Does `{code}` turn off certificate, host name or signature verification?",
    yes: "It turns off certificate or host name checks, accepts invalid certificates or host names, or decodes a signed token such as a JWT without verifying its signature.",
    no: "It keeps verification on, or makes no TLS connection and reads no signed token.",
    no_examples: &[],
};

pub(super) const DJANGO_CORS: Check = Check {
    id: "cors",
    question: "Does `{code}` set cross-origin rules that let pages from origins it does not fully check read the deployed site's responses with credentials?",
    yes: "It allows any origin, reflects the request's origin, or matches origins loosely, such as by suffix or substring, while allowing credentials, in code or settings that the deployed site uses.",
    no: concat!(
        "It allows only listed origins by exact match or allows no credentials; it sets no cross-origin rules itself, ",
        "whatever the site's settings choose; or a settings module for production that imports these settings sets it again."
    ),
    no_examples: &[],
};

pub(super) const DJANGO_COOKIE: Check = Check {
    id: "cookie",
    question: "Does `{code}` set or configure a session or authentication cookie of the deployed site without the Secure or HttpOnly flag?",
    yes: "A cookie that holds a session or token is set or configured without Secure or without HttpOnly, in code or settings that the deployed site uses.",
    no: concat!(
        "Such cookies have both flags, the cookie holds no session or token, or the code sets no cookie; ",
        "or a settings module for production that imports these settings sets it again."
    ),
    no_examples: &[],
};

pub(super) const DEBUG: Check = Check {
    id: "debug",
    question: "Does `{code}` turn on a web framework's debug mode or detailed error pages for the deployed site?",
    yes: "It turns debug mode on, such as DEBUG = True, in code or settings that the deployed site uses.",
    no: "Debug mode is off, is read from the environment with off as the default, is turned on only in settings for tests or local development, or a settings module for production that imports these settings sets it again.",
    no_examples: &[],
};

pub(super) const CSRF: Check = Check {
    id: "csrf",
    question: "Does `{code}` turn off protection against cross-site request forgery for requests that change data?",
    yes: "A view or route that changes data as the user its session cookie signs in, such as their profile, password or records, is exempted from the CSRF check, such as with csrf_exempt, or the CSRF middleware or check is removed.",
    no: "CSRF protection stays on; the exempted endpoint authenticates each request itself rather than with the session cookie, such as a webhook that verifies a signature, an API that reads a token from a header, or a form for visitors who are not signed in that asks for a password reset email or checks a reset token it is sent; it only reads data; or it sets nothing about CSRF.",
    no_examples: &[],
};

pub(super) const SECRET: Check = Check {
    id: "literal_secret",
    question: "Does `{code}` set a signing key, password or token that the deployed site uses to a literal written in the code?",
    yes: "A secret key, password, token or API key that the deployed program uses is a literal in the code, including one shown as a redacted literal.",
    no: "Secrets are read from the environment, a file or a secret store; the literal is empty or only a placeholder; it is used only in tests or local development; or a settings module for production that imports these settings sets it again.",
    no_examples: &[],
};

pub(super) const DJANGO_EXCEPTION_TO_CLIENT: Check = Check {
    id: "exception_to_client",
    question: "Does `{code}` send an exception's message, stack trace or a database error to a remote client in a response?",
    yes: "The text of an exception it did not raise itself to explain bad input, or a stack trace, is put into the response to a request.",
    no: "Responses carry fixed messages or codes, or only messages written to explain invalid input, such as those of Django's or a form's ValidationError; exceptions it does not catch go to the framework's error handling; details stay in server logs.",
    no_examples: &[],
};

pub(super) const ENVIRONMENT_TO_CLIENT: Check = Check {
    id: "environment_to_client",
    question: "Does `{code}` send the server's environment variables, settings or whole request metadata to a remote client?",
    yes: "It puts the process environment, the application's settings, or a whole request metadata object such as Django's request.META, which holds the server's environment, into a response or a page it renders.",
    no: "It sends only chosen fields meant for the client, such as the user's own name or a public setting, or sends no such data.",
    no_examples: &[],
};

pub(super) const DJANGO_REDIRECT: Check = Check {
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
