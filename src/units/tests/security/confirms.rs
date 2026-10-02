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
            Detail::Security { confirms, .. } => confirms.checked.as_ref().map(|c| c.request()),
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
            .map(|f| (composed(f), f.message.clone()))
    };
    assert_eq!(
        judged(choice_of("outside", &PATHS)).map(|f| f.0),
        Some(Strength::Review)
    );
    assert_eq!(
        judged(choice_of("confined", &PATHS)),
        None,
        "a UUID cannot climb out of the directory: a note, not reported"
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
    let when = ["always", "debug", "none", "opt_in", "output"];
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
    assert_eq!(status, Status::Clear);
    assert!(message.is_empty(), "a note, not reported: {message}");
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
            .map(composed)
    };
    let markup = MARKUP_VALUES;
    assert_eq!(
        judged(
            ENCODED,
            "markup",
            "markup_values",
            choice_of("raw", &markup)
        ),
        Some(Strength::Review)
    );
    assert_eq!(
        judged(
            ENCODED,
            "markup",
            "markup_values",
            choice_of("encoded", &markup)
        ),
        None,
        "percent-encoded before the link: a note, not reported"
    );
    let reach = REACH;
    assert_eq!(
        judged(
            ADMIN,
            "redirect",
            "redirect_reach",
            choice_of("anywhere", &reach)
        ),
        Some(Strength::Review)
    );
    assert_eq!(
        judged(
            ADMIN,
            "redirect",
            "redirect_reach",
            choice_of("own_site", &reach)
        ),
        None,
        "the admin path comes first: a note, not reported"
    );
}

/// The options of the Choice on what a markup finding's values hold.
pub(super) const MARKUP_VALUES: [&str; 5] = ["encoded", "own", "raw", "typed", "unknown"];

/// The options of the Choice on where a redirect finding's targets lead.
pub(super) const REACH: [&str; 4] = ["anywhere", "checked", "none", "own_site"];

/// The options of the Choice on what an SQL, command or code finding's
/// values hold where they enter it.
pub(super) const QUERY_VALUES: [&str; 6] = ["allowed", "fixed", "own", "raw", "typed", "unknown"];

/// A route building `ORDER BY` from one of two clauses its preset selects.
pub(super) const PRESET: &str = "fn sorted(conn: &Connection, preset: &str) -> Result<Vec<i64>> {\n    let order = match preset {\n        \"speed\" => \"speed ASC\",\n        \"rank\" => \"rank ASC\",\n        _ => return Err(unknown()),\n    };\n    conn.query(&format!(\"SELECT id FROM models ORDER BY {order}\"))\n}\n";

#[test]
fn a_query_finding_is_a_note_when_its_values_are_fixed_text() {
    let judged = |choice: Value| {
        let (project, options) = security_project(PRESET);
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("interpreted", noul_at(0.95)),
            ("sql", noul_at(0.95)),
            ("origin", spread(0.0, 0.0, 1.0)),
            ("query_values", choice),
        ];
        first_of(&run(&project, &options, &mut eval), "security/injection")
    };
    review_until_harmless(
        judged,
        (&QUERY_VALUES, "fixed"),
        "one of two clauses written in the code",
    );
}

/// The composed level of the first finding of `rule` in a report of one file.
fn first_of(report: &Report, rule: &str) -> Option<Strength> {
    report.files[0]
        .findings
        .iter()
        .find(|f| f.rule == rule)
        .map(composed)
}

/// A finding that a confirm Choice follows stays a review when the Choice
/// answers `raw`, and is a note, which one level does not report, when it
/// answers `harmless`.
fn review_until_harmless(
    judged: impl Fn(Value) -> Option<Strength>,
    (options, harmless): (&[&str], &str),
    reason: &str,
) {
    assert_eq!(judged(choice_of("raw", options)), Some(Strength::Review));
    assert_eq!(judged(choice_of(harmless, options)), None, "{reason}");
}

#[test]
fn a_markup_consider_on_parameters_is_a_note_when_its_values_arrive_escaped() {
    let judged = |choice: Value| {
        let (project, options) = security_project(ENCODED);
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("interpreted", noul_at(0.95)),
            ("markup", noul_at(0.95)),
            ("origin", spread(0.0, 1.0, 0.0)),
            ("values", choice_of("outside", &VALUES)),
            ("markup_values", choice),
        ];
        let report = run(&project, &options, &mut eval);
        report.files[0]
            .findings
            .iter()
            .find(|f| f.rule == "security/injection")
            .map(composed)
    };
    assert_eq!(
        judged(choice_of("raw", &MARKUP_VALUES)),
        Some(Strength::Consider)
    );
    assert_eq!(
        judged(choice_of("encoded", &MARKUP_VALUES)),
        None,
        "text another party wrote, escaped before it enters the markup"
    );
}

/// A React component writing a syntax highlighter's output as raw HTML.
pub(super) const HIGHLIGHTED: &str = "export function CodeBlock({ code }) {\n  const html = highlight(code)\n  return <code dangerouslySetInnerHTML={{ __html: html }} />\n}\n";

#[test]
fn unescaped_html_is_a_note_when_the_library_that_built_it_escaped_it() {
    let judged = |choice: Value| {
        let (report, _) = settings_run(
            "code-block.jsx",
            HIGHLIGHTED,
            &[("weakened", 0.95), ("escape", 0.95)],
            Some(("raw_html", choice)),
        );
        first_of(&report, "security/unsafe-settings")
    };
    review_until_harmless(
        judged,
        (&MARKUP_VALUES, "encoded"),
        "the highlighter escapes the code",
    );
}

/// The options of the Choice on who reads a function's error text.
pub(super) const READERS: [&str; 5] = ["local", "operator", "own_services", "public", "unknown"];

#[test]
fn error_details_only_their_own_user_reads_are_a_note_asked_with_the_readme() {
    let (project, mut options) = security_project(QUERY);
    project.write(
        "README.md",
        "# Proxy\n\n[![CI](https://example.org/badge.svg)](https://example.org)\n<img src=\"logo.png\">\n\nA proxy you run on your own machine for your coding agent.\n",
    );
    let boundary = |deny: &[&str]| {
        let config = crate::config::Config {
            upload_deny: deny.iter().map(|d| d.to_string()).collect(),
            ..Default::default()
        };
        crate::boundary::Boundary::new(&config).unwrap()
    };
    assert_eq!(
        crate::docs::project_opening(&project.0, &boundary(&["README.md"])),
        None,
        "a README the upload boundary denies is not sent"
    );
    options.project = crate::docs::project_opening(&project.0, &boundary(&[]));
    assert_eq!(
        options.project.as_deref(),
        Some("# Proxy\nA proxy you run on your own machine for your coding agent."),
        "badges and HTML are left out"
    );
    let (_, plan) = planned(&project, &options);
    let readers = plan.files[&0]
        .units
        .iter()
        .find_map(|u| match &u.detail {
            Detail::Security { confirms, .. } => confirms.readers.as_ref().map(|c| c.request()),
            _ => None,
        })
        .expect("a sensitive-data unit asked who reads its errors");
    assert_eq!(
        readers["state"]["project"]["readme_opening"],
        json!(options.project)
    );
    let mut judged = |chosen: &str| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("error_details", noul_at(0.95)),
            ("destination", to_client().1),
            ("error_readers", choice_of(chosen, &READERS)),
        ];
        let report = run(&project, &options, &mut eval);
        options.refresh = true;
        report.files[0]
            .findings
            .iter()
            .find(|f| f.rule == "security/sensitive-data")
            .map(composed)
    };
    assert_eq!(judged("public"), Some(Strength::Review));
    assert_eq!(
        judged("local"),
        None,
        "the person running it reads its logs anyway: a note, not reported"
    );
}

#[test]
fn a_value_shown_as_the_output_its_user_asked_for_is_no_logged_secret() {
    let (project, options) = security_project(TOKENS);
    let logs = [
        "plain", "identity", "operator", "secret", "personal", "none",
    ];
    let when = ["always", "debug", "none", "opt_in", "output"];
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("logs_secret", noul_at(0.95)),
        ("logs_object_secret", noul_at(0.95)),
        ("logged", choice_of("secret", &logs)),
        ("logged_when", choice_of("output", &when)),
    ];
    let report = run(&project, &options, &mut eval);
    assert!(
        !report.files[0]
            .findings
            .iter()
            .any(|f| f.rule == "security/sensitive-data"),
        "shown only to the person who asked: a note, not reported"
    );
}
