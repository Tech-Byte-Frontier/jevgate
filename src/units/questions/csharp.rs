//! How questions read about C# files. ASP.NET Core and .NET name what a
//! question asks about with their own APIs (`FromSqlInterpolated`,
//! `UseDeveloperExceptionPage`, `TokenValidationParameters`), and some C#
//! idioms need an example of the answer they take. These additions extend
//! the general criteria only in requests about C# files, and the checks
//! below are asked only there: every other language keeps the wording
//! measured on its own projects.
use super::Check;
use serde_json::{Value, json};

/// The language whose files these additions reword, as a file's state names it.
pub const CSHARP: &str = "C#";

/// What a question adds about C# files: a question asked in its place (with
/// `{code}` for the state path it names), and examples of each answer. For a
/// score, the examples of "yes" extend its top level.
struct Wording {
    id: &'static str,
    question: Option<&'static str>,
    yes: &'static [&'static str],
    no: &'static [&'static str],
}

/// Measured on eShopOnWeb and dvcsharp-api. Controller actions whose
/// parameters were placed into SQL had no caller to settle their origin and
/// split between "parameters" and "another party"; functions that catch no
/// exception leaned toward sending error details, since an exception they
/// let through reaches the client through ASP.NET Core's own handling; an
/// exception middleware that writes `exception.Message` stayed at 0.78; a
/// reset key hashed from an email address was not "made with a random
/// generator"; and token options (`ValidateIssuer`, `RequireHttpsMetadata`)
/// leaned toward "turns off certificate verification". Seeding methods
/// hold sample rows and accounts by design, and read as unexplained values
/// and special-cased users.
const WORDING: [Wording; 16] = [
    Wording {
        id: "interpreted",
        question: Some(
            "Does `{code}` place a variable into the text of a database query, shell command, code to evaluate or HTML markup, or into the type of objects it creates?",
        ),
        yes: &[
            "SQL built with interpolation or concatenation and passed to FromSqlRaw, ExecuteSqlRaw, SqlCommand or Dapper",
            "A type name read from input passed to Type.GetType or Activator.CreateInstance, or to a serializer that then creates objects of that type",
        ],
        no: &[
            "An interpolated string written directly as the argument of FromSqlInterpolated or ExecuteSqlInterpolated, which turn each value into a parameter",
            "Deserializing into a type fixed in the code, such as JsonSerializer.Deserialize<Order> or new XmlSerializer(typeof(Product[]))",
        ],
    },
    Wording {
        id: "error_details",
        question: None,
        yes: &[],
        no: EXCEPTIONS_PASSED_ON,
    },
    Wording {
        id: "weakened",
        question: None,
        yes: &[
            "Detailed error pages or debug mode turned on for a deployed application",
            "Token signature or expiry checks turned off",
            "A signing or encryption key written in the code",
        ],
        no: &[],
    },
    Wording {
        id: "origin",
        question: Some(
            "Where do the variables that `{code}` places into a query, command, code, markup, file path, URL or type name come from?",
        ),
        yes: &[
            "This includes parameters a web framework fills from the request, such as those of a controller action marked [HttpGet] or [HttpPost] or of a route handler.",
        ],
        no: &[],
    },
    Wording {
        id: "sql",
        question: None,
        yes: &[
            "An interpolated or concatenated string, or a variable holding one, passed to FromSqlRaw, ExecuteSqlRaw, Entity Framework Core 2's FromSql, SqlCommand or Dapper",
        ],
        no: &[
            "An interpolated string written directly as the argument of FromSqlInterpolated, ExecuteSqlInterpolated or Entity Framework Core's FromSql and SqlQuery, which turn each value into a parameter",
        ],
    },
    Wording {
        id: "shell",
        question: None,
        yes: &[
            "Process.Start of cmd.exe /c or /bin/sh -c with a command line that holds a variable",
        ],
        no: &["Process.Start with a ProcessStartInfo whose ArgumentList holds each argument"],
    },
    Wording {
        id: "tls",
        question: None,
        yes: &[
            "A certificate validation callback, such as ServerCertificateCustomValidationCallback or RemoteCertificateValidationCallback, that returns true",
        ],
        no: &[
            "Options of security tokens such as ValidateIssuer or ValidateAudience, which check claims rather than certificates",
            "RequireHttpsMetadata = false, which allows token metadata over plain HTTP but leaves certificate checks on",
        ],
    },
    Wording {
        id: "random",
        question: Some(
            "Does `{code}` make a token, code, password or identifier that must be unguessable with a non-cryptographic random generator, or from data others can know?",
        ),
        yes: &[
            "System.Random used for a reset code, token or password",
            "A reset key or token derived from an email address, user id, name or time, for example by hashing it",
        ],
        no: &[
            "RandomNumberGenerator, or a random value that is not a secret",
            "Calling another function of the program that makes the value, which is judged itself",
        ],
    },
    Wording {
        id: "hash",
        question: None,
        yes: &["MD5.Create, SHA1.Create or SHA256.HashData applied to a password"],
        no: &[
            "PasswordHasher, BCrypt or Rfc2898DeriveBytes with many iterations",
            "Calling another function of the program that hashes the password, which is judged itself",
        ],
    },
    Wording {
        id: "cors",
        question: None,
        yes: &[
            "AllowAnyOrigin, SetIsOriginAllowed(_ => true) or a reflected origin together with AllowCredentials",
        ],
        no: &["WithOrigins listing exact origins, or AllowAnyOrigin without AllowCredentials"],
    },
    Wording {
        id: "cookie",
        question: None,
        yes: &[
            "CookieOptions or cookie authentication options that set HttpOnly = false or Secure = false, or CookieSecurePolicy.None, for a session or token cookie",
        ],
        no: &[],
    },
    Wording {
        id: "exception_to_client",
        question: None,
        yes: &[],
        no: EXCEPTIONS_PASSED_ON,
    },
    Wording {
        id: "own_messages",
        question: None,
        yes: &["It throws no exception and catches none, so no error text passes through it"],
        no: &[],
    },
    Wording {
        id: "handler_leaks",
        question: None,
        yes: &[
            "The Message of every exception it catches, such as exception.Message written to the response for any exception, which includes library and database errors",
        ],
        no: &[],
    },
    Wording {
        id: "magic",
        question: None,
        yes: &[],
        no: &[
            "Rows of seed or sample data that a seeding method inserts, such as product names, prices and image paths",
        ],
    },
    Wording {
        id: "special",
        question: None,
        yes: &[],
        no: &[
            "A seeding method that creates sample records, a demo user or the first administrator account",
        ],
    },
];

/// Exceptions a function lets through reach the client only through the
/// framework's handling, which is judged as an error handler of its own.
const EXCEPTIONS_PASSED_ON: &[&str] = &[
    "Exceptions it lets propagate without catching them: ASP.NET Core's exception handling writes those responses and is judged itself",
    "A controller action or startup code that catches no exception and puts no exception's text into what it returns",
    "A developer exception page turned on only when the environment is Development",
];

/// Injection checks asked only about C# files.
pub const CSHARP_UNHANDLED: [Check; 1] = [Check {
    id: "type",
    question: "Does `{code}` create objects of a type that a variable names, or deserialize data that names its own types?",
    yes: "A type name from a variable reaches Type.GetType, Activator.CreateInstance or a serializer, or it deserializes with BinaryFormatter, NetDataContractSerializer, LosFormatter, SoapFormatter or Json.NET with TypeNameHandling other than None.",
    no: "Every type it creates or deserializes into is fixed in the code or checked against an allowed list, or it creates no objects from data.",
    no_examples: &[],
}];

/// Weak settings asked only about C# files: developer exception pages,
/// token validation and signing keys written in the code, which the broad
/// question found in ASP.NET Core setups and no other check names. Turning off only the issuer or audience check
/// is not among them: tokens the program signs itself carry its own claims.
pub const CSHARP_SETTINGS: [Check; 3] = [
    Check {
        id: "debug",
        question: "Does `{code}` show detailed error pages, stack traces or debugging tools to the users of a deployed application?",
        yes: "It turns on a developer exception page, detailed errors or debug tools without limiting them to development, such as UseDeveloperExceptionPage outside an IsDevelopment check.",
        no: "Such pages and tools are on only when the environment is Development, or it turns on none.",
        no_examples: &[],
    },
    Check {
        id: "token",
        question: "Does `{code}` accept security tokens without verifying their signature or expiry?",
        yes: "It turns off the signature or lifetime check of JSON Web Tokens or other signed tokens, such as ValidateIssuerSigningKey = false, RequireSignedTokens = false, ValidateLifetime = false or the algorithm none, or trusts a token or cookie that it only decodes.",
        no: "Signatures and expiry are checked where tokens are accepted, or it accepts none: it only stores or sends a token, or checks that one is present, and a server verifies it.",
        no_examples: &[
            "Turning off only the issuer or audience check of tokens the program signs itself",
        ],
    },
    Check {
        id: "key",
        question: "Does `{code}` sign or encrypt with a key or secret written in the code?",
        yes: "A signing or encryption key, such as the secret of a SymmetricSecurityKey that signs or validates JSON Web Tokens, is a string or bytes written in the code or a constant of the program.",
        no: "Keys are read from configuration, the environment or a secret store, or it uses no key.",
        no_examples: &[],
    },
];

/// Reword the body of question `id` for a file in `language`: unchanged
/// except in C# files.
pub fn reword(language: &str, id: &str, body: &mut Value) {
    if language != CSHARP {
        return;
    }
    let Some(wording) = WORDING.iter().find(|w| w.id == id) else {
        return;
    };
    if let Some(question) = wording.question {
        let asked = body["instructions"]["question"].as_str().unwrap_or("");
        let code = asked.split('`').nth(1).unwrap_or("");
        body["instructions"]["question"] = json!(question.replace("{code}", code));
    }
    if body["type"] == "score" {
        if let Some(top) = body["criteria"].as_array_mut().and_then(|l| l.last_mut())
            && !wording.yes.is_empty()
        {
            let text = top.as_str().unwrap_or("");
            *top = json!(format!("{text} {}", wording.yes.join(" ")));
        }
        return;
    }
    add_examples(&mut body["criteria"]["true"], wording.yes);
    add_examples(&mut body["criteria"]["false"], wording.no);
}

/// Extend an answer's examples, turning a plain criterion into one with examples.
fn add_examples(criterion: &mut Value, examples: &[&str]) {
    if examples.is_empty() {
        return;
    }
    if let Some(what) = criterion.as_str() {
        *criterion = json!({"what": what, "examples": []});
    }
    if let Some(list) = criterion["examples"].as_array_mut() {
        list.extend(examples.iter().map(|e| json!(e)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::questions;

    #[test]
    fn csharp_files_get_framework_examples_and_other_languages_keep_their_wording() {
        let general = questions::security_weakened("functions[0].source");
        let mut python = general.clone();
        reword("Python", "weakened", &mut python);
        assert_eq!(python, general);
        let mut csharp = general.clone();
        reword(CSHARP, "weakened", &mut csharp);
        let examples = csharp["criteria"]["true"]["examples"].as_array().unwrap();
        assert_eq!(examples.len(), 8);
        assert!(
            examples[7]
                .as_str()
                .unwrap()
                .starts_with("A signing or encryption key")
        );

        let mut sql = questions::UNHANDLED[0].body("function.source");
        reword(CSHARP, "sql", &mut sql);
        assert_eq!(
            sql["criteria"]["true"]["what"],
            questions::UNHANDLED[0].body("function.source")["criteria"]["true"]
        );
        assert!(
            sql["criteria"]["false"]["examples"][0]
                .as_str()
                .unwrap()
                .contains("FromSqlInterpolated")
        );

        let mut random = questions::WEAK_SETTINGS[2].body("module.source");
        reword(CSHARP, "random", &mut random);
        let question = random["instructions"]["question"].as_str().unwrap();
        assert!(
            question.starts_with("Does `module.source` make a token"),
            "{question}"
        );
        assert!(question.ends_with("from data others can know?"));

        let mut origin = questions::security_origin("function.source", true);
        reword(CSHARP, "origin", &mut origin);
        assert!(
            origin["criteria"][2]
                .as_str()
                .unwrap()
                .ends_with("[HttpPost] or of a route handler.")
        );
        assert_eq!(
            origin["criteria"][0],
            questions::security_origin("function.source", true)["criteria"][0]
        );
        assert!(
            origin["instructions"]["question"]
                .as_str()
                .unwrap()
                .contains("or type name")
        );
    }

    #[test]
    fn csharp_questions_are_short_and_name_a_state_path() {
        let checks = CSHARP_UNHANDLED.iter().chain(&CSHARP_SETTINGS);
        let mut bodies: Vec<Value> = checks.map(|c| c.body("function.source")).collect();
        for wording in WORDING.iter().filter_map(|w| w.question) {
            bodies.push(
                json!({"instructions": {"question": wording.replace("{code}", "function.source")}}),
            );
        }
        for body in bodies {
            let text = body["instructions"]["question"].as_str().unwrap();
            assert!(text.ends_with('?') && text.len() < 200, "{text}");
            assert!(text.contains("`function.source`"), "{text}");
        }
    }
}
