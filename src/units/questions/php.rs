//! How security questions read about PHP files. A PHP page writes its
//! response with `echo`, runs shells with backticks and `shell_exec`,
//! includes files by path and escapes SQL values with its database driver,
//! so its questions name those functions; the checks and Choices below are
//! asked only there. Every other language keeps the wording measured on its
//! own projects.
use super::{Check, EVIDENCE};
use serde_json::{Value, json};

/// The language whose files these questions reword, as a file's state names it.
pub const PHP: &str = "PHP";

/// A question as it reads about PHP files, in place of the general one: its
/// question (with `{code}` for the state path it names) and both answers.
struct Wording {
    id: &'static str,
    question: &'static str,
    yes: &'static str,
    no: &'static str,
    no_examples: &'static [&'static str],
}

/// Measured on DVWA and Slim-Skeleton. Asked only about binding, DVWA's
/// escaped and quoted guestbook inserts were SQL reviews, so the SQL check
/// counts driver escaping inside quotes and numbers as handled. The path
/// check names a file picked from fixed names, as page scripts include their
/// parts; the URL check tells a request the code sends from a redirect, a
/// link or a host passed to a shell command, which pages also build; the
/// presence questions name `echo`, `include`, `Location` headers and
/// `unserialize`, and error details name `die(mysqli_error(…))`. Laravel's
/// `Str::random` draws from `random_bytes`: BookStack's token and
/// generated-password functions stayed near 0.4 on the random check.
const WORDING: [Wording; 17] = [
    Wording {
        id: "interpreted",
        question: "Does `{code}` place a variable into the text of a database query, shell command, code to evaluate, or HTML markup, or deserialize a variable's value into objects?",
        yes: "A variable is joined, formatted or interpolated into the text of a query, command, code or markup that is then run, rendered or written into a page with echo, print or <?= ?>, or a variable is passed to unserialize.",
        no: "Variables are passed only as bound parameters, separate arguments, or through a template or component that escapes them; the text is built only from fixed values; data is parsed only as JSON or another plain format; or the function builds no such text.",
        no_examples: &[],
    },
    Wording {
        id: "resource",
        question: "Does `{code}` open, include, write or request a file path or URL, or redirect to a URL, that is taken or built from a variable?",
        yes: "The path of a file it opens, runs with include or require, writes or deletes, the URL it requests, or the URL it redirects the client to, comes from or is built with a variable such as a parameter or a value read from input.",
        no: "Every path and URL it uses is fixed in the code or read from the program's configuration, or it uses none.",
        no_examples: &[],
    },
    Wording {
        id: "error_details",
        question: "Does `{code}` put internal error details, such as stack traces, database errors or server paths, into the response it sends to a remote client?",
        yes: "It puts an exception's stack trace, a database or library error message, a query, or a server path into the response to a request from a remote client, such as by echoing it into the page or passing it to die or exit, or turns on display_errors.",
        no: "It returns generic messages or error codes, keeps details in server logs or sends them to its own error-reporting service, shows errors to the local user of a command-line or desktop program, or sends no response.",
        no_examples: OWN_EXCEPTIONS,
    },
    Wording {
        id: "exception_to_client",
        question: "Does `{code}` send an exception's message, stack trace or a database error to a remote client in a response?",
        yes: "The text of an exception the program did not write itself, such as a PDOException, a database or library error, or a stack trace, is put into the response to a request, echoed into the page or passed to die or exit.",
        no: "Responses carry fixed messages or codes, or only messages the program wrote itself; details stay in server logs.",
        no_examples: OWN_EXCEPTIONS,
    },
    Wording {
        id: "sql",
        question: "Does `{code}` put a variable into the text of an SQL query without binding it, escaping it inside quotes, or converting it to a number?",
        yes: "A variable that is neither escaped by the database driver inside quotes nor converted to a number is joined or interpolated into SQL text that is then run, or an escaped value is placed outside quotes.",
        no: "Values are passed as bound parameters or placeholders, escaped with the driver's escape function (such as mysqli_real_escape_string or PDO::quote) and placed inside quotes, or converted to numbers with intval or a cast; identifiers come from a fixed list or the database schema; or it runs no SQL.",
        no_examples: &[],
    },
    Wording {
        id: "shell",
        question: "Does `{code}` run a shell command string that holds a variable?",
        yes: "A command line built with a variable is run through a shell, such as with exec, shell_exec, system, passthru, popen, proc_open or backticks.",
        no: "It quotes each variable for the shell with escapeshellarg, runs only fixed commands, accepts only values it has checked against a strict format, or runs no command.",
        no_examples: &[
            "A command built only from parts each checked with is_numeric, ctype_digit, filter_var or an anchored regular expression before it runs",
        ],
    },
    Wording {
        id: "code",
        question: "Does `{code}` evaluate text that holds a variable as PHP code?",
        yes: "It passes text that holds a variable to eval, assert, create_function or preg_replace with the /e modifier.",
        no: "It evaluates no text as PHP code, or only fixed code; a shell command, SQL query, markup or included file is not code it evaluates.",
        no_examples: &[],
    },
    Wording {
        id: "markup",
        question: "Does `{code}` put a variable into HTML markup without escaping it?",
        yes: "A variable is joined or interpolated into HTML text, or written into the page with echo, print or <?= ?>, without htmlspecialchars, htmlentities or another escaping function.",
        no: "Values go through htmlspecialchars, htmlentities or a template that escapes them, are numbers or values checked against fixed choices, or it builds no markup.",
        no_examples: &[
            "A variable that already holds HTML the program built elsewhere, such as a page body or a form a helper returns, joined into a larger page",
        ],
    },
    Wording {
        id: "path",
        question: "Does `{code}` open, include, write or delete a file at a path built from a variable without checking that it stays inside a directory?",
        yes: "A path is built from a variable that can hold a name or path from outside the program, such as a request, upload or user input, and is opened, run with include or require, written or deleted without reducing it to a base name, rejecting parent-directory parts, or checking it against a list of allowed files.",
        no: "Such paths are checked; are joined only from fixed text, __DIR__ and constants the program defines with define or const; are built from the program's own directories joined with names the program chooses, such as a file picked from fixed names by a switch; come from the program's configuration; or it uses no such path.",
        no_examples: &[],
    },
    Wording {
        id: "url",
        question: "Does `{code}` request a URL or host taken from a variable without checking the host?",
        yes: "It sends a request to a URL or host that comes from a variable, such as with curl, file_get_contents or fopen, without checking the host against an allowed list or rejecting private addresses.",
        no: "The host is fixed, comes from the program's configuration or the command line of the person running a local script (getopt or $argv), or is checked; or it requests no URL. A redirect it sends the client, a link it writes into the page, or a host it passes to a shell command is not a request it sends.",
        no_examples: &[],
    },
    Wording {
        id: "redirect",
        question: "Does `{code}` redirect the client to a URL or path taken from a variable without checking where it leads?",
        yes: "A URL or path that a request carries, such as a query parameter, form field, header or cookie, is passed to a redirect, such as a Location header, redirect() or a redirect response, without checking that it is a path on the program's own site or that its host is on an allowed list; checking only that it contains, starts or ends with some text is no such check.",
        no: "The target is fixed, is chosen from fixed targets, is checked to be a path on its own site or a host on an allowed list, comes from the program's configuration or a field of an exception or object the program set itself, or is a destination a signed-in user saved on purpose, such as a link they attached; or it does not redirect.",
        no_examples: &[],
    },
    Wording {
        id: "tls",
        question: "Does `{code}` turn off certificate or host name verification?",
        yes: "It turns off certificate or host name checks, such as CURLOPT_SSL_VERIFYPEER set to false, CURLOPT_SSL_VERIFYHOST set to 0, or verify_peer set to false in a stream context.",
        no: "It keeps verification on, turns it off only when an administrator sets an option of the deployment's configuration for it, or makes no TLS connection.",
        no_examples: &[],
    },
    Wording {
        id: "hash",
        question: "Does `{code}` hash passwords or derive keys from them with a fast or broken hash, or with few iterations?",
        yes: "It hashes passwords or derives keys from them with md5, sha1, crypt with a weak salt, a single round of SHA-256 through hash, or hash_pbkdf2 with few iterations.",
        no: "It uses password_hash and password_verify, or hash_pbkdf2 with many iterations, or it does not handle passwords.",
        no_examples: CALLED,
    },
    Wording {
        id: "random",
        question: "Does `{code}` make a token, code, password or identifier that must be unguessable with a non-cryptographic generator or from a predictable value?",
        yes: "It makes a secret value, such as a session id, token, reset or verification code, or random password, with rand, mt_rand, uniqid or lcg_value, or from a counter, the time or a hash of such values.",
        no: "It uses random_bytes, random_int, openssl_random_pseudo_bytes or Laravel's Str::random, which is built on random_bytes, or the value is not a secret.",
        no_examples: CALLED,
    },
    Wording {
        id: "cookie",
        question: "Does `{code}` set or configure a session or authentication cookie without the Secure or HttpOnly flag?",
        yes: "It sets a cookie that holds a session or token with setcookie or session_set_cookie_params, or starts a session after setting session.cookie_httponly or session.cookie_secure off, without Secure or without HttpOnly.",
        no: "Such cookies have both flags, the cookie holds no session or token, or the code sets no cookie.",
        no_examples: CALLED,
    },
    Wording {
        id: "logs_secret",
        question: "Does `{code}` write a password, token, key or personal data to a log or the server's console?",
        yes: "It writes a password, token, API key, session identifier, or personal data about a person such as an email address to a log with error_log, syslog, a logger or a log file, or to the server's console.",
        no: "It logs only messages, identifiers that are not secret such as record ids, counts, or errors without such values, or it logs nothing. Text written into the page with echo, print or <?= ?> is the response, not a log.",
        no_examples: &[],
    },
    Wording {
        id: "logs_object_secret",
        question: "Does `{code}` log an object, configuration, request, command or list of arguments that holds a password, token or key?",
        yes: "It writes a whole object, configuration, request, command line or argument list that holds a password, token, key or secret to a log with error_log, syslog, a logger or a log file.",
        no: "It logs only values that hold no secret, masks secrets before logging, or logs nothing. Text written into the page with echo, print, print_r or var_dump is the response, not a log.",
        no_examples: &[],
    },
];

/// What the origin question counts in PHP files: BookStack's function that
/// writes an uploaded image's contents to a path it is given was answered
/// for the upload (0.93), not for the path.
const ORIGIN: &str = "Only variables placed into that text or path count: data written into a file or a database, such as an uploaded file's contents, does not.";

/// A Laravel controller that returns the message of its own
/// `FileUploadException`, caught by name, sends text the program wrote
/// ("File path … could not be uploaded to"); on BookStack four such
/// controllers were error-detail reviews.
const OWN_EXCEPTIONS: &[&str] = &[
    "The message of an exception class the program defines for its own errors, caught by that class's name, such as catch (FileUploadException $e), which carries messages the program writes; PHP's Exception, PDOException or a library's exceptions are not such classes",
];

/// A function that only calls another function of the program to make a
/// token, hash a password or start a session: DVWA's pages call
/// `generateSessionToken()` and `dvwa_start_session()`, and their random
/// and cookie checks stayed near 0.3 and 0.4.
const CALLED: &[&str] = &[
    "Calling another function of the program that makes the value, hashes the password or sets the cookie, such as generateSessionToken() or a session helper, which is judged itself",
];

/// Examples a question adds about PHP files, where its general wording
/// stays: (question id, examples of "yes", examples of "no").
const EXAMPLES: [(&str, &[&str], &[&str]); 1] = [(
    "weakened",
    &[
        "md5 or sha1 of a password, rand, mt_rand or uniqid for a session token, CURLOPT_SSL_VERIFYPEER set to false, or a session cookie without httponly",
    ],
    CALLED,
)];

/// Injection checks asked only about PHP files, and only of source that
/// names what they ask about: page scripts `unserialize` cookies and move
/// uploaded files themselves. A page that only showed an upload form was
/// asked whether it saves uploads, and stayed near 0.4.
pub const PHP_UNHANDLED: [Check; 2] = [
    Check {
        id: "deserialize",
        question: "Does `{code}` pass a variable to unserialize without restricting the classes it may create?",
        yes: "It passes a variable to unserialize without the allowed_classes option set to false or a list of classes.",
        no: "It parses JSON or another plain data format, passes allowed_classes, or deserializes nothing.",
        no_examples: &[],
    },
    Check {
        id: "upload",
        question: "Does `{code}` save an uploaded file under a name or extension the uploader chose, without checking the extension against a list of allowed types?",
        yes: "It keeps the uploaded file's own name or extension, or checks only the type the client declared, and saves it where the web server may serve or run it.",
        no: "It allows only listed extensions or checks the content, names saved files itself, stores uploads outside the directories the server serves, hands the file to a storage service or disk of the program whose location this code does not show, or saves no uploads.",
        no_examples: &[],
    },
];

/// What source must name, in any case, for a PHP check to be asked of it.
const MENTIONS: [(&str, &[&str]); 2] = [
    ("deserialize", &["unserialize"]),
    ("upload", &["$_files", "move_uploaded_file", "uploadedfile"]),
];

/// Whether `source` names what the PHP check `id` asks about.
pub fn php_mentions(id: &str, source: &str) -> bool {
    let source = source.to_ascii_lowercase();
    MENTIONS
        .iter()
        .find(|(check, _)| *check == id)
        .is_none_or(|(_, names)| names.iter().any(|name| source.contains(name)))
}

/// Options of the markup Choice that rule a markup check out: escaped or
/// fixed values, text the program itself produces, markup built elsewhere,
/// element data, or no HTML.
pub const HANDLED_MARKUP: [&str; 5] = ["escaped", "internal", "built", "data", "none"];

/// What a PHP function joins into HTML unescaped, asked whenever its markup
/// check is not clear. Page scripts append to a page body the included file
/// built, and markdown libraries fill element arrays that another function
/// escapes; asked whether a variable reaches markup unescaped, both stayed
/// near 0.5. Pages that read a request also join ids converted to numbers,
/// command output or database errors, and the markup check found those at
/// 0.9 while the origin question answered for the request. Naming what is
/// joined decided both. With `callers`, a page template such as
/// `dvwaHtmlEcho($page)` shows that its parameter is the page it prints.
pub fn security_markup_parts(code: &str, callers: bool) -> Value {
    let (parameter, built, note) = if callers {
        (
            "A parameter or other value whose origin neither this code nor `callers` shows, holding text such as a name, message, link or id rather than HTML.",
            "Only HTML that other code of the program builds, such as a page body, a form or a fragment a helper returns, held in a variable an included file sets or passed by `callers` in a parameter meant to hold HTML; that code is judged where it builds it.",
            format!("`callers` holds functions that call it. {EVIDENCE}"),
        )
    } else {
        (
            "A parameter or other value whose origin this code does not show, holding text such as a name, message, link or id rather than HTML.",
            "Only HTML that other code of the program builds, such as a page body, a form or a fragment a helper returns, held in a variable an included file sets or passed in a parameter meant to hold HTML; that code is judged where it builds it.",
            EVIDENCE.to_string(),
        )
    };
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("What does `{code}` join into HTML text without escaping it?"),
            "note": note,
        },
        "criteria": {
            "request": "A value this code reads from the request, a cookie or an uploaded file, such as $_GET, $_POST, $_COOKIE, $_SERVER or $_FILES, or text derived from one, other than a number.",
            "stored": "A field of a database record or a file that users can write, such as a name, comment or message.",
            "parameter": parameter,
            "escaped": "Only values passed through htmlspecialchars, htmlentities or another escaping function, numbers such as ids converted with intval, and values checked against fixed choices.",
            "internal": "Only text the program or the server produces itself, such as fixed messages, error messages, command output, dates, configuration, the client's address, or files that ship with the program such as its documentation.",
            "built": built,
            "data": "No HTML text: it only puts values, parameters included, into arrays or objects that describe elements, such as a name, text and attributes, which other code turns into HTML.",
            "none": "It builds or writes no HTML: it writes nothing, or only JSON or plain text.",
        },
    })
}

/// Options of the markup Choice that name values another party controls:
/// a markup check that found a variable is then a review whatever the
/// origin question said about the other values of a page.
pub const OUTSIDE_MARKUP: [&str; 2] = ["request", "stored"];

/// Options of the shell Choice that rule a shell check out.
pub const CHECKED_COMMANDS: [&str; 3] = ["checked", "internal", "none"];

/// What the command lines a PHP function runs hold, asked whenever its shell
/// check is not clear: DVWA's page that splits an address into four octets,
/// checks each with is_numeric and joins them again was a command injection
/// at 0.88, while the pages that only strip `&&` or `;` are real ones.
pub fn security_shell_parts(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("What do the command lines that `{code}` runs hold besides fixed text?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "request": "A value from the request, a cookie or an uploaded file, used as received or after only removing or replacing some characters or words.",
            "stored": "A field of a database record or a file that users can write.",
            "parameter": "A parameter or other value whose origin this code does not show, used without such a check.",
            "checked": "Only values quoted with escapeshellarg, numbers, values chosen from fixed choices, or values rebuilt from parts that are each checked against a strict format before the command runs, such as parts that each pass is_numeric or an anchored regular expression.",
            "internal": "Only constants, configuration or values the program produces itself.",
            "none": "It runs no command.",
        },
    })
}

/// Options of the path Choice that rule an undecided path check out.
pub const FIXED_PATHS: [&str; 2] = ["fixed", "none"];

/// Where the paths a PHP function opens or includes come from, asked when
/// its path check stays undecided: pages include their parts through a
/// directory constant and a file name a switch picks, and the path check
/// stayed near 0.25 on them. Names stored in a database or file are their
/// own option, since a general question about paths read such names as the
/// program's own and cleared real traversals. It can only clear.
pub fn security_path_parts(code: &str) -> Value {
    json!({
        "type": "choice",
        "instructions": {
            "question": format!("Where do the file paths that `{code}` opens, includes, writes or deletes come from?"),
            "note": EVIDENCE,
        },
        "criteria": {
            "fixed": "Only fixed text, __DIR__, constants the program defines, the program's configuration, and file names it picks from fixed choices, such as by a switch.",
            "request": "Partly from the request, a cookie or an uploaded file, such as $_GET or $_FILES.",
            "stored": "Partly from a database record or a file's content.",
            "parameter": "Partly from a parameter or a variable whose origin this code does not show, such as one an included file sets.",
            "none": "It opens, includes, writes or deletes no file.",
        },
    })
}

/// Reword the body of question `id` for a file in `language`: unchanged
/// except in PHP files, where a Noul with a PHP wording reads it instead.
pub fn reword_php(language: &str, id: &str, body: &mut Value) {
    if language != PHP {
        return;
    }
    if id == "origin" {
        let note = body["instructions"]["note"].as_str().unwrap_or("");
        body["instructions"]["note"] = json!(format!("{ORIGIN} {note}").trim_end().to_string());
        return;
    }
    if body["type"] != "noul" {
        return;
    }
    if let Some((_, yes, no)) = EXAMPLES.iter().find(|(question, ..)| *question == id) {
        for (answer, examples) in [("true", yes), ("false", no)] {
            if let Some(list) = body["criteria"][answer]["examples"].as_array_mut() {
                list.extend(examples.iter().map(|e| json!(e)));
            }
        }
        return;
    }
    let Some(wording) = WORDING.iter().find(|w| w.id == id) else {
        return;
    };
    let asked = body["instructions"]["question"].as_str().unwrap_or("");
    let code = asked.split('`').nth(1).unwrap_or("").to_string();
    body["instructions"]["question"] = json!(wording.question.replace("{code}", &code));
    body["criteria"]["true"] = json!(wording.yes);
    body["criteria"]["false"] = if wording.no_examples.is_empty() {
        json!(wording.no)
    } else {
        json!({"what": wording.no, "examples": wording.no_examples})
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::questions;

    #[test]
    fn php_files_get_their_own_wording_and_other_languages_keep_theirs() {
        let general = questions::UNHANDLED[0].body("function.source");
        let mut python = general.clone();
        reword_php("Python", "sql", &mut python);
        assert_eq!(python, general);
        let mut php = general.clone();
        reword_php(PHP, "sql", &mut php);
        let text = php.to_string();
        assert!(text.contains("mysqli_real_escape_string"), "{text}");
        assert!(!text.contains("Drizzle"), "{text}");
        assert!(
            php["instructions"]["question"]
                .as_str()
                .unwrap()
                .starts_with("Does `function.source` put a variable")
        );

        let mut markup = questions::UNHANDLED[3].body("module.source");
        reword_php(PHP, "markup", &mut markup);
        assert!(
            markup["criteria"]["false"]["examples"][0]
                .as_str()
                .unwrap()
                .starts_with("A variable that already holds HTML")
        );
        assert!(
            markup["instructions"]["question"]
                .as_str()
                .unwrap()
                .contains("`module.source`")
        );

        // Questions without a PHP wording, and Choices, keep the general one.
        let messages = questions::security_own_messages("function.source");
        let mut php_messages = messages.clone();
        reword_php(PHP, "own_messages", &mut php_messages);
        assert_eq!(php_messages, messages);
        let origin = questions::security_origin("function.source", false, false);
        let mut php_origin = origin.clone();
        reword_php(PHP, "origin", &mut php_origin);
        assert_eq!(php_origin["criteria"], origin["criteria"]);
        assert!(
            php_origin["instructions"]["note"]
                .as_str()
                .unwrap()
                .starts_with("Only variables placed into that text or path count")
        );

        // The broad setting question keeps its wording and gains examples.
        let weakened = questions::security_weakened("function.source", false);
        let mut php_weakened = weakened.clone();
        reword_php(PHP, "weakened", &mut php_weakened);
        assert_eq!(
            php_weakened["instructions"], weakened["instructions"],
            "same question"
        );
        let handled = php_weakened["criteria"]["false"]["examples"]
            .as_array()
            .unwrap();
        assert!(
            handled
                .last()
                .and_then(Value::as_str)
                .unwrap()
                .contains("generateSessionToken()")
        );
        let mut logs = questions::security_logs_secret("function.source");
        reword_php(PHP, "logs_secret", &mut logs);
        assert!(logs["criteria"]["false"].as_str().unwrap().contains("echo"));
    }

    #[test]
    fn php_checks_are_asked_only_of_source_that_names_them() {
        assert!(php_mentions(
            "deserialize",
            "$u = UNSERIALIZE($_COOKIE['c']);"
        ));
        assert!(!php_mentions("deserialize", "echo json_decode($body);"));
        assert!(php_mentions("upload", "move_uploaded_file($tmp, $dest);"));
        assert!(php_mentions("upload", "$f = $_FILES['uploaded'];"));
        assert!(!php_mentions(
            "upload",
            "<form enctype=\"multipart/form-data\">"
        ));
        assert!(php_mentions("sql", "anything"));
    }

    #[test]
    fn php_questions_are_short_and_name_a_state_path() {
        let mut bodies: Vec<Value> = PHP_UNHANDLED
            .iter()
            .map(|c| c.body("function.source"))
            .collect();
        bodies.push(security_markup_parts("function.source", false));
        bodies.push(security_markup_parts("function.source", true));
        bodies.push(security_path_parts("function.source"));
        bodies.push(security_shell_parts("function.source"));
        for wording in &WORDING {
            bodies.push(
                json!({"instructions": {"question": wording.question.replace("{code}", "function.source")}}),
            );
        }
        for body in bodies {
            let text = body["instructions"]["question"].as_str().unwrap();
            assert!(text.ends_with('?') && text.len() < 200, "{text}");
            assert!(text.contains("`function.source`"), "{text}");
        }
    }
}
