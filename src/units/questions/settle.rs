//! The settle Choices of security units: one literal Choice per kind of
//! check, naming what the code does (where its URLs, redirect targets and
//! text go, how it renders markup and handles tokens and passwords, what its
//! logs write, which origins and cookies it allows), asked apart from the
//! checks; some options clear the check they settle.
use super::{EVIDENCE, security::CALLERS};
use serde_json::{Value, json};

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

/// The option of the runs-in Choice that rules a forged request out.
pub const BROWSER: &str = "browser";

/// Where a function runs, asked when the URL check stays undecided, since a
/// request from the user's browser reaches only what that user can. Offered
/// beside the URL's parts, the browser lost to "a whole URL handed to it"
/// for a client component's fetch helper.
pub fn security_runs_in(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("Where does `{code}` run once the program is deployed?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "browser": "Only in the user's web browser: in a client component, in a web page script, or in a component or hook that only client code uses.",
            "server": "On a server or in a backend process: a route handler, server component, Server Action, API, job, or command-line tool.",
            "either": "Either side may run it, such as shared code that both server and browser code import, or the code does not show which.",
        },
    })
}

/// Options of the redirect-target Choice that rule an open redirect out.
pub const OWN_TARGETS: [&str; 3] = ["own", "checked", "none"];

/// Where the targets a function redirects clients to come from, asked when
/// the redirect check stays undecided: client components that navigate to
/// fixed paths or to a checkout URL their server returns, and helpers that
/// build a path their callers name, split on "a URL or path taken from a
/// variable". Offered "a whole path handed to it" beside "what callers
/// pass", helpers whose callers pass fixed paths took the first, which is
/// true as well; with callers shown, that option is only for paths the
/// callers do not explain.
pub fn security_redirect_target(code: &str, callers: bool) -> Value {
    let (own, given, note) = if callers {
        (
            "A path or URL written in the code, built from the program's own origin or configuration, or returned by the program's own server code or a service it calls, such as a payment provider's checkout page, in the function or in what `callers` pass it; variables fill only ids, names, numbers or messages in its segments or query.",
            "A whole path or URL handed to the function as a parameter, where `callers` does not show where it comes from.",
            format!("{CALLERS} {EVIDENCE}"),
        )
    } else {
        (
            "A path or URL written in the code, built from the program's own origin or configuration, or returned by the program's own server code or a service it calls, such as a payment provider's checkout page; variables fill only ids, names, numbers or messages in its segments or query.",
            "A whole path or URL handed to the function as a parameter or field.",
            EVIDENCE.to_string(),
        )
    };
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("Where do the paths or URLs that `{code}` redirects or navigates the client to come from?"),
            "note": note,
        },
        "criteria": {
            "own": own,
            "checked": "A path or URL from a variable that is checked before the redirect to be a path on the program's own site or on a host from an allowed list.",
            "given": given,
            "outside": "A whole path or URL that a request carries, such as a query parameter, form field, header or cookie, or an argument of a function clients call directly, without such a check.",
            "none": "It redirects or navigates nowhere; it only builds or returns a path, or it has no redirect.",
        },
    })
}

/// Options of the markup Choice that rule a markup injection out.
pub const INERT_MARKUP: [&str; 3] = ["escaped", "text", "none"];

/// How the markup a function builds with variables is rendered, asked when
/// the markup check stays undecided: React components with values in
/// attributes, and snippets shown in a text field, split on "a variable put
/// into markup without escaping". A Django view is asked what it sends
/// back: views that only redirect or render a template split on the markup
/// check, since the variables they pass on end up in a page, and a template
/// escapes them unless it writes one with `|safe`.
pub fn security_markup_output(code: &str, django: bool) -> Value {
    if django {
        return json!({
            "type": "choice",
            "instructions": {
                "question": format!("What does `{code}` send back to the client, and how are the variables in it rendered?"),
                "note": EVIDENCE,
            },
            "criteria": {
                "escaped": "A page rendered from a template that writes each value it is given without a safe filter or autoescaping off, which Django escapes, or HTML built with format_html or escape.",
                "text": "It is never rendered as HTML: JSON, a file download or plain text.",
                "raw": "HTML it builds from variables as text itself, text it marks safe with mark_safe, or a template that writes a value it is given with a safe filter or with autoescaping off.",
                "none": "No markup with variables: it only redirects, or sends nothing to a client itself.",
            },
        });
    }
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("How is the markup that `{code}` builds with variables rendered?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "escaped": "By JSX or a template engine that escapes each value: variables appear only as element children, attribute values or component props, or go through an escaping or sanitizing function first.",
            "text": "It is never rendered as HTML: it is shown as plain text, such as a code snippet in a text field, or sent as text.",
            "raw": "As raw HTML with a variable inside, unescaped: through dangerouslySetInnerHTML, innerHTML, insertAdjacentHTML, document.write, an iframe srcdoc, or an HTML response built as text.",
            "none": "It builds no HTML or SVG markup with variables.",
        },
    })
}

/// Options of the logging Choice that rule a logged secret out.
pub const PLAIN_LOGS: [&str; 4] = ["plain", "identity", "operator", "none"];

/// What a function's log statements write, asked whenever a logging signal
/// is not clear: an error caught from a payment or database call, logged
/// with a message, split on the check for a logged object; and the question
/// whether it logs personal data found an audit line naming who signed in
/// (vaultwarden's "User {email} logged in successfully. IP: {ip}") and a
/// command printing recovery codes for the admin who ran it: 10 of 19
/// labeled logging reviews were such lines.
pub fn security_logged(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("What do the log and console statements of `{code}` write?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "plain": "Only messages, ids, counts, statuses, or an error caught from a failed call, none of which holds a password, token or key.",
            "identity": "Who did what: a user's id, name, email address or IP address beside the action they took, as an audit or access log records, and no secret.",
            "operator": "Values it shows on purpose to the person running a command-line tool, such as recovery codes or credentials a command prints for that person.",
            "secret": "A password, token, API key or other secret, or a whole object, configuration, request or argument list that holds one.",
            "personal": "Other personal data about a person, such as a home address, document number, or health or payment details.",
            "none": "It logs or prints nothing.",
        },
    })
}

/// Options of the token Choice that rule an unverified-token concern out.
pub const VERIFIED_TOKENS: [&str; 5] = [
    "verifies",
    "passes",
    "verified_before",
    "reads_claims",
    "none",
];

/// What a function does with security tokens, asked whenever the token check
/// is not clear: front-end hooks that read their own token to send it and
/// middleware that looks a session up stayed between 0.2 and 0.5 on the
/// check, while naming what the code does with tokens decides. Reading a
/// token's claims is apart from deciding access with them: code that read
/// the expiry of a token its identity provider had just sent, or the
/// character id of an access token, was chosen as trusting it unverified.
pub fn security_token_use(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("What does `{code}` do with security tokens, such as JSON Web Tokens or session tokens?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "verifies": "It verifies each token's signature and expiry, or looks the token up in its own store, before trusting what it holds.",
            "passes": "It only creates, signs, stores, sends or forwards tokens, or checks that one is present, while a server verifies them.",
            "verified_before": "It reads the claims of a token verified before it runs, such as by middleware, or of a token it has just received from an identity provider over TLS.",
            "reads_claims": "It decodes a token only to read or show what it says, such as a user id, a name or its expiry, while other code or a server decides what the caller may do.",
            "decides_access": "It decides what the caller may do, such as signing them in, granting a role or accepting a reset, from a token it has not verified.",
            "turned_off": "It turns off a check a library makes by default, such as verify_signature=False, verify=False, an algorithm list that allows none, or ignoreExpiration.",
            "none": "It handles no security tokens.",
        },
    })
}

/// Options of the password Choice that rule a weak-password concern out.
pub const HASHED_PASSWORDS: [&str; 2] = ["slow_hash", "none"];

/// How a function treats users' passwords, asked whenever the password
/// check is not clear: HMAC signing, key loading and a demo login form were
/// reviews or stayed between 0.2 and 0.4 on it.
pub fn security_password_handling(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("How does `{code}` handle users' passwords?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "slow_hash": "It hashes them with bcrypt, scrypt, Argon2 or a key derivation function with many iterations, or hands them to a library, framework or model hook that does.",
            "plain": "It saves them, or checks a login against saved ones, as plain text.",
            "fast_hash": "It hashes them with MD5, SHA-1, a single round of SHA-256 or another fast hash, or derives keys from them with few iterations.",
            "none": "It stores and checks no users' passwords: what it hashes, signs or encrypts is other data, such as tokens, messages, files or keys, or it only fills in or sends a password someone types.",
        },
    })
}

/// Options of the CORS Choice that rule a credentialed-origin concern out.
pub const SAFE_ORIGINS: [&str; 3] = ["unset", "listed", "public"];

/// Which other sites a function lets send credentialed requests, asked when
/// the CORS check stays undecided: route handlers that set cookies or answer
/// preflights with `*` and no credentials split on "any origin allowed".
pub fn security_cors_origins(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("Which other sites does `{code}` let send requests that carry a user's cookies or credentials?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "unset": "None: it sets no CORS header or option.",
            "listed": "Only origins written in the code or configuration, or the program's own origin.",
            "public": "Any origin, but without allowing credentials: no `Access-Control-Allow-Credentials: true` or credentials option, as for a public or token-authenticated API.",
            "any": "Any origin, or whatever origin a request names reflected back, with credentials allowed.",
        },
    })
}

/// The options of the cookie Choice that clear the cookie check.
pub const FLAGGED_COOKIES: [&str; 2] = ["unset", "flagged"];

/// What a function leaves a session cookie's flags as, asked when the cookie
/// check stays undecided: a SvelteKit form action's `cookies.set` without
/// options, whose defaults set both flags, stayed at 0.21.
pub fn security_cookie_flags(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("How are the Secure and HttpOnly flags set on the cookies `{code}` sets?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "unset": "It sets no cookie, or only cookies that hold no session, token or sign-in state, such as a theme or language preference.",
            "flagged": "Session or token cookies get both flags: in the options it passes, or from a framework whose defaults set them, such as SvelteKit's `cookies.set`.",
            "missing": "A session or token cookie is set with Secure or HttpOnly turned off, or through an API whose defaults leave them off, such as Express `res.cookie`, `document.cookie` or PHP `setcookie` without them.",
        },
    })
}

/// The options of the destination Choice that rule error details out: every
/// place but a remote client.
pub const AWAY_FROM_CLIENTS: [&str; 4] = ["local", "logs", "caller", "stored"];

/// Where a function's text goes, asked when an error-detail signal stays
/// undecided. An error or body shaped for a response counts as the client:
/// helpers that format errors for a server's callers return them. A game
/// client that hands the server's error text to its own window over a
/// channel of `Response` messages was answered as sending it to a client,
/// so the local option names the program's own screens.
pub fn security_destination(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("Where does the text that `{code}` produces or passes on go?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "client": "Into a response to a request from another computer: an HTTP, API or RPC response, a message to a connected remote client, or an error, status or body shaped for such a response that it builds or returns.",
            "local": "To the person running a local program: a terminal, console or window, the program's own screens that a desktop, game or mobile app reaches through a channel, event or IPC call, or a report or file on their own machine.",
            "logs": "To logs, or to the program's own error reporting or monitoring.",
            "caller": "Back to the code that called it as an ordinary error or value, such as a parse, lookup or validation failure, not shaped as a response.",
            "stored": "Into a database, queue, cache or job record.",
        },
    })
}
