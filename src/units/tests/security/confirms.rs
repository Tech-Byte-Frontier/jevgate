//! The Choices asked after a finding: what a path can hold, when a log line
//! runs, and what markup holds or where a redirect leads.
use super::*;

/// A Rocket route joining an id its type parses as a UUID to a directory.
pub(super) const DOWNLOAD: &str = "#[derive(Clone, UuidFromParam)]\npub struct FileId(String);\n\n#[get(\"/files/<id>\")]\nasync fn download(id: FileId) -> Option<NamedFile> {\n    let path = Path::new(\"data\").join(id.as_ref());\n    NamedFile::open(path).await.ok()\n}\n";

/// The options of the Choice on what a path finding's paths can hold.
pub(super) const PATHS: [&str; 5] = ["confined", "local", "outside", "own", "unknown"];

#[test]
fn a_path_finding_is_a_note_when_its_paths_stay_in_their_directory() {
    let (project, mut options) = security_project(DOWNLOAD);
    let (_, plan) = planned(&project, &options);
    let paths = plan.files[&0]
        .units
        .iter()
        .find_map(|u| match &u.detail {
            Detail::Security {
                checked: Some(checked),
                ..
            } => Some(checked.request()),
            _ => None,
        })
        .expect("an injection unit with a confirm of its checks");
    for question in ["paths", "markup_values", "redirect_reach"] {
        assert!(paths["questions"].get(question).is_some(), "{question}");
    }
    assert!(
        paths["state"]["types_named_in_parameters"][0]
            .as_str()
            .unwrap()
            .contains("UuidFromParam"),
        "the parameter's type is shown with its derive list: {paths}"
    );
    let mut judged = |choice: Value| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("resource", noul_at(0.95)),
            ("path", noul_at(0.95)),
            ("origin", spread(0.0, 0.0, 1.0)),
            ("paths", choice),
        ];
        let report = run(&project, &options, &mut eval);
        options.refresh = true;
        report.files[0]
            .findings
            .iter()
            .find(|f| f.rule == "security/injection")
            .map(|f| (f.strength, f.message.clone()))
    };
    assert_eq!(
        judged(choice_of("outside", &PATHS)).map(|f| f.0),
        Some(Strength::Review)
    );
    let (strength, message) = judged(choice_of("confined", &PATHS)).unwrap();
    assert_eq!(
        strength,
        Strength::Note,
        "a UUID cannot climb out of the directory"
    );
    assert!(
        message.contains("likely keeps the path inside"),
        "{message}"
    );
}

/// Tokens logged only under a setting that exists to log them.
pub(super) const TOKENS: &str = "fn exchange(token: &str) -> String {\n    if CONFIG.sso_debug_tokens() {\n        debug!(\"Access token {token}\");\n    }\n    token.to_string()\n}\n";

#[test]
fn a_log_line_an_operator_turns_on_to_log_tokens_is_a_note() {
    let (project, mut options) = security_project(TOKENS);
    let logs = [
        "plain", "identity", "operator", "secret", "personal", "none",
    ];
    let when = ["always", "debug", "none", "opt_in"];
    let mut judged = |chosen: &str| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("logs_secret", noul_at(0.95)),
            ("logs_object_secret", noul_at(0.95)),
            ("logged", choice_of("secret", &logs)),
            ("logged_when", choice_of(chosen, &when)),
        ];
        let report = run(&project, &options, &mut eval);
        options.refresh = true;
        let file = &report.files[0];
        let messages: String = file.findings.iter().map(|f| f.message.clone()).collect();
        (
            file.dimensions[catalog::SENSITIVE_DATA].status.clone(),
            messages,
        )
    };
    assert_eq!(
        judged("debug").0,
        Status::Review,
        "debug level is still a log"
    );
    let (status, message) = judged("opt_in");
    assert_eq!(status, Status::Note);
    assert!(
        message.contains("only when an operator turns on"),
        "{message}"
    );
}

/// A handler that percent-encodes a name before it builds a link, and one
/// that redirects to its admin path followed by a form value.
pub(super) const ENCODED: &str = "fn breach(username: &str) -> String {\n    let name: String = form_urlencoded::byte_serialize(username.as_bytes()).collect();\n    format!(\"<a href=\\\"https://example.org/?q={name}\\\">{name}</a>\")\n}\n";
pub(super) const ADMIN: &str = "fn login(form: Form<Login>) -> Redirect {\n    let target = form.redirect.clone();\n    Redirect::to(format!(\"{}{target}\", admin_path()))\n}\n";

#[test]
fn markup_and_redirect_findings_are_notes_when_their_values_can_do_no_harm() {
    let judged = |source: &str, check: &'static str, question: &'static str, choice: Value| {
        let (project, options) = security_project(source);
        let mut eval = scripted(0);
        let presence = if check == "markup" {
            "interpreted"
        } else {
            "resource"
        };
        eval.overrides = vec![
            (presence, noul_at(0.95)),
            (check, noul_at(0.95)),
            ("origin", spread(0.0, 0.0, 1.0)),
            (question, choice),
        ];
        let report = run(&project, &options, &mut eval);
        report.files[0]
            .findings
            .iter()
            .find(|f| f.rule == "security/injection")
            .map(|f| (f.strength, f.message.clone()))
            .unwrap()
    };
    let markup = MARKUP_VALUES;
    assert_eq!(
        judged(
            ENCODED,
            "markup",
            "markup_values",
            choice_of("raw", &markup)
        )
        .0,
        Strength::Review
    );
    let (strength, message) = judged(
        ENCODED,
        "markup",
        "markup_values",
        choice_of("encoded", &markup),
    );
    assert_eq!(strength, Strength::Note, "percent-encoded before the link");
    assert!(message.contains("escaped or encoded before"), "{message}");
    let reach = REACH;
    assert_eq!(
        judged(
            ADMIN,
            "redirect",
            "redirect_reach",
            choice_of("anywhere", &reach)
        )
        .0,
        Strength::Review
    );
    let (strength, message) = judged(
        ADMIN,
        "redirect",
        "redirect_reach",
        choice_of("own_site", &reach),
    );
    assert_eq!(strength, Strength::Note, "the admin path comes first");
    assert!(message.contains("keeps it on the site"), "{message}");
}

/// The options of the Choice on what a markup finding's values hold.
pub(super) const MARKUP_VALUES: [&str; 5] = ["encoded", "own", "raw", "typed", "unknown"];

/// The options of the Choice on where a redirect finding's targets lead.
pub(super) const REACH: [&str; 4] = ["anywhere", "checked", "none", "own_site"];
