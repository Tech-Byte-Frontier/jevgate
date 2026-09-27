//! The settle Choices: for each kind of check that can stay undecided after the
//! trace and recheck, the one question that settles it, and the units it is
//! asked for.
use super::*;

/// A Choice asked when one of a rule's checks stays undecided after the
/// trace and recheck; it can only clear the checks it settles, so it is
/// asked apart from them.
pub(in crate::units) struct SettleKind {
    pub rule: &'static str,
    /// The question id of the Choice.
    pub question: &'static str,
    /// The checks that call for it while undecided, and that it settles.
    pub checks: &'static [&'static str],
    /// The options whose combined probability at the threshold clears them.
    pub clears: &'static [&'static str],
    /// Whether the functions that call the subject are sent with it.
    callers: bool,
    pub when: SettleWhen,
    /// The files whose units it is planned for.
    files: SettleFiles,
}

/// The files, by language, whose units a settle Choice is planned for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettleFiles {
    All,
    Only(&'static str),
    Except(&'static str),
}

impl SettleFiles {
    fn include(self, language: &str) -> bool {
        match self {
            SettleFiles::All => true,
            SettleFiles::Only(only) => language == only,
            SettleFiles::Except(except) => language != except,
        }
    }
}

/// Which units a settle Choice is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::units) enum SettleWhen {
    /// An uncertain unit whose checks stay undecided.
    Undecided,
    /// Also a consider or note that rests on its undecided checks: a note
    /// that a client component's fetch "places a parameter into a URL it
    /// requests" only puzzled readers.
    UndecidedOrFinding,
    /// Any unit whose checks are not clear, and it clears a check that found
    /// a concern too: a PHP page joins into HTML the body its included file
    /// built, ids converted to numbers and database errors, and the markup
    /// check found those at 0.9 while the origin question answered for the
    /// request the page also reads.
    NotClear,
}

/// Every settle Choice. Where a URL comes from settles the URL check: on
/// clients of a fixed or configured service it split on a variable path or
/// query; so does code that runs only in the user's browser. Where a redirect leads, how markup is rendered and which origins
/// may send credentials settle theirs, which split on client components that
/// navigate to fixed paths or render values as attributes, and on route
/// handlers that answer preflights for any origin without credentials. What
/// its logs write settles its logging signals whenever they are not clear:
/// a logged object split on errors caught from a payment or database call,
/// and audit lines naming who signed in were logged personal data. Where a
/// function's text goes is asked whenever its error-detail signals are not
/// clear, too: a game client handing the server's error text to its own
/// window over a channel whose messages are named `Response` was fifteen
/// reviews for sending details to a remote client. Where a function's text goes settles error
/// details (see `exposure_signal`), also under a finding that claims the text
/// likely reaches a client.
///
/// PHP pages ask what they join into HTML in place of how markup is
/// rendered, which names JSX and client components, and where the paths
/// they open or include come from: pages include their parts through a
/// directory constant and a file name a switch picks, and the path check
/// stayed near 0.25 on them. Both are asked whenever their check is not
/// clear: a page whose markup check leaned toward a concern was a note, so
/// its undecided path Choice was never asked, and once the markup Choice
/// cleared the markup it was left uncertain. What their command lines hold
/// settles the shell check the same way: a page that checks each octet of
/// an address with is_numeric was a command injection at 0.88. What code
/// does with tokens and how it handles passwords settle those checks
/// whenever they are not clear: front ends that send their own token and
/// HMAC signing split on them or were reviews.
pub(in crate::units) const SETTLES: [SettleKind; 14] = [
    SettleKind {
        rule: INJECTION,
        question: "url_parts",
        checks: &["url"],
        clears: &questions::OWN_PARTS,
        callers: true,
        when: SettleWhen::Undecided,
        files: SettleFiles::All,
    },
    SettleKind {
        rule: INJECTION,
        question: "runs_in",
        checks: &["url"],
        clears: &[questions::BROWSER],
        callers: false,
        when: SettleWhen::UndecidedOrFinding,
        files: SettleFiles::All,
    },
    SettleKind {
        rule: INJECTION,
        question: "redirect_target",
        checks: &["redirect"],
        clears: &questions::OWN_TARGETS,
        callers: true,
        when: SettleWhen::Undecided,
        files: SettleFiles::All,
    },
    SettleKind {
        rule: INJECTION,
        question: "markup_output",
        checks: &["markup"],
        clears: &questions::INERT_MARKUP,
        callers: false,
        when: SettleWhen::Undecided,
        files: SettleFiles::Except(questions::PHP),
    },
    SettleKind {
        rule: INJECTION,
        question: "markup_parts",
        checks: &["markup"],
        clears: &questions::HANDLED_MARKUP,
        callers: true,
        when: SettleWhen::NotClear,
        files: SettleFiles::Only(questions::PHP),
    },
    SettleKind {
        rule: INJECTION,
        question: "shell_parts",
        checks: &["shell"],
        clears: &questions::CHECKED_COMMANDS,
        callers: false,
        when: SettleWhen::NotClear,
        files: SettleFiles::Only(questions::PHP),
    },
    SettleKind {
        rule: INJECTION,
        question: "path_parts",
        checks: &["path"],
        clears: &questions::FIXED_PATHS,
        callers: false,
        when: SettleWhen::NotClear,
        files: SettleFiles::Only(questions::PHP),
    },
    SettleKind {
        rule: INJECTION,
        question: "path_source",
        checks: &["path"],
        clears: &questions::OWN_PATHS,
        callers: true,
        when: SettleWhen::Undecided,
        files: SettleFiles::Except(questions::PHP),
    },
    SettleKind {
        rule: SENSITIVE_DATA,
        question: "destination",
        checks: &["error_details", "exception_to_client"],
        clears: &questions::AWAY_FROM_CLIENTS,
        callers: false,
        when: SettleWhen::NotClear,
        files: SettleFiles::All,
    },
    SettleKind {
        rule: SENSITIVE_DATA,
        question: "logged",
        checks: &["logs_object_secret", "logs_secret"],
        clears: &questions::PLAIN_LOGS,
        callers: false,
        when: SettleWhen::NotClear,
        files: SettleFiles::All,
    },
    SettleKind {
        rule: UNSAFE_SETTINGS,
        question: "cors_origins",
        checks: &["cors"],
        clears: &questions::SAFE_ORIGINS,
        callers: false,
        when: SettleWhen::Undecided,
        files: SettleFiles::All,
    },
    SettleKind {
        rule: UNSAFE_SETTINGS,
        question: "cookie_flags",
        checks: &["cookie"],
        clears: &questions::FLAGGED_COOKIES,
        callers: false,
        when: SettleWhen::Undecided,
        files: SettleFiles::All,
    },
    SettleKind {
        rule: UNSAFE_SETTINGS,
        question: "token_use",
        checks: &["token"],
        clears: &questions::VERIFIED_TOKENS,
        callers: false,
        when: SettleWhen::NotClear,
        files: SettleFiles::All,
    },
    SettleKind {
        rule: UNSAFE_SETTINGS,
        question: "password_handling",
        checks: &["hash"],
        clears: &questions::HASHED_PASSWORDS,
        callers: false,
        when: SettleWhen::NotClear,
        files: SettleFiles::All,
    },
];

/// The settle follow-ups of one unit, one per Choice of its rule, each sent
/// only when its checks stay undecided.
pub(super) fn settles(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    rule: &'static str,
    id: &str,
) -> Vec<Settle> {
    SETTLES
        .iter()
        .filter(|kind| kind.rule == rule && kind.files.include(file.language))
        .filter_map(|kind| {
            let request = settle(file, subject, kind, id);
            file.budget.fits(&request.0).then_some(Settle {
                question: kind.question,
                request: request.into(),
            })
        })
        .collect()
}

pub(super) fn settle(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    kind: &SettleKind,
    id: &str,
) -> (Value, Asked) {
    let code = subject.code();
    let callers = kind.callers && !subject.callers.is_empty();
    let body = match kind.question {
        "url_parts" => questions::security_url_parts(&code, callers),
        "path_source" => questions::security_path_source(&code, callers),
        "runs_in" => questions::security_runs_in(&code),
        "redirect_target" => questions::security_redirect_target(&code, callers),
        "markup_output" => {
            questions::security_markup_output(&code, subject.django, subject.renders())
        }
        "markup_parts" => questions::security_markup_parts(&code, callers),
        "path_parts" => questions::security_path_parts(&code),
        "shell_parts" => questions::security_shell_parts(&code),
        "destination" => questions::security_destination(&code),
        "logged" => questions::security_logged(&code),
        "cookie_flags" => questions::security_cookie_flags(&code),
        "token_use" => questions::security_token_use(&code),
        "password_handling" => questions::security_password_handling(&code),
        _ => questions::security_cors_origins(&code),
    };
    let mut questions = Questions::default();
    questions.ask(
        kind.question.into(),
        body,
        id,
        kind.rule,
        kind.question,
        Pass::Settle,
    );
    let mut state = json!({
        "file": file.file_state(),
        subject.kind: subject.state(),
    });
    if callers {
        state["callers"] = json!(
            subject
                .callers
                .iter()
                .map(|(name, source)| json!({"name": name, "source": source}))
                .collect::<Vec<_>>()
        );
    }
    file.request("settle", state, questions)
}
