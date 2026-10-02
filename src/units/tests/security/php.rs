//! PHP page scripts: a file's top-level statements as one unit, the checks
//! named in PHP's own functions, and the Choices that settle a page.
use super::*;

pub(super) const PAGE: &str = "<?php\nrequire_once 'lib.php';\n\nfunction escape_html($text) {\n    return htmlspecialchars($text, ENT_QUOTES);\n}\n\nif (isset($_GET['id'])) {\n    $id = $_GET['id'];\n    $query = \"SELECT name FROM users WHERE id = '$id'\";\n    $result = mysqli_query($db, $query);\n    echo '<p>' . $_GET['name'] . '</p>';\n}\n?>\n<footer><?= date('Y') ?></footer>\n";

#[test]
fn a_php_page_script_is_one_unit_judged_by_every_security_rule() {
    let (project, options) = project_with(&[("page.php", PAGE)], &catalog::SECURITY);
    let (_, plan) = planned(&project, &options);
    let request = &plan.requests[0].request;
    let functions = request["state"]["functions"].as_array().unwrap();
    let names: Vec<&str> = functions
        .iter()
        .filter_map(|f| f["name"].as_str())
        .collect();
    assert_eq!(names, ["escape_html", "top-level code"]);
    let script = functions[1]["source"].as_str().unwrap();
    assert!(script.starts_with("require_once 'lib.php';\nif (isset($_GET['id']))"));
    assert!(!script.contains("function escape_html") && !script.contains("<footer>"));
    assert!(script.ends_with("date('Y')"), "{script}");
    let asked: Vec<&String> = request["questions"].as_object().unwrap().keys().collect();
    for question in ["f1_interpreted", "f1_error_details", "f1_weakened"] {
        assert!(asked.iter().any(|q| *q == question), "{asked:?}");
    }

    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.95)),
        ("sql", noul_at(0.97)),
        ("markup", noul_at(0.9)),
        ("origin", spread(0.0, 0.05, 0.95)),
        ("markup_parts", choice_of("request", &MARKUP_PARTS)),
    ];
    let report = run(&project, &options, &mut eval);
    let finding = report.files[0]
        .findings
        .iter()
        .find(|f| f.rule == "security/injection")
        .unwrap();
    assert_eq!(finding.symbol.as_deref(), Some("top-level code"));
    assert_eq!(finding.category.as_deref(), Some("CWE-89 SQL injection"));
    assert!(
        finding.message.starts_with(
            "Top-level code places values from another party into a database query and markup without"
        ),
        "{}",
        finding.message
    );
    assert_eq!(
        (
            finding.locations.last().unwrap().start_line,
            finding.locations.last().unwrap().end_line
        ),
        (2, 15)
    );
}

#[test]
fn php_checks_name_php_functions_and_its_own_kinds_only_where_the_source_names_them() {
    let rust = traced_checks("lib.rs", QUERY);
    for kind in ["deserialize", "upload"] {
        assert!(!rust.contains_key(kind), "{kind} in {rust:?}");
    }
    assert!(!rust["sql"].to_string().contains("mysqli"));
    let plain = traced_checks(
        "page.php",
        "<?php\n$id = $_GET['id'];\n$rows = mysqli_query($db, \"SELECT * FROM t WHERE id = $id\");\n",
    );
    assert!(
        plain["sql"]
            .to_string()
            .contains("mysqli_real_escape_string")
    );
    assert!(plain["redirect"].to_string().contains("Location header"));
    for kind in ["deserialize", "upload"] {
        assert!(!plain.contains_key(kind), "{kind} is asked only when named");
    }
    let named = traced_checks(
        "page.php",
        "<?php\nheader('Location: ' . $_GET['next']);\n$p = unserialize($_COOKIE['p']);\nmove_uploaded_file($_FILES['f']['tmp_name'], 'up/x');\n",
    );
    for kind in ["redirect", "deserialize", "upload"] {
        assert!(named.contains_key(kind), "{kind} in {named:?}");
    }
}

pub(super) const MARKUP_PARTS: [&str; 8] = [
    "request",
    "stored",
    "parameter",
    "escaped",
    "internal",
    "built",
    "data",
    "none",
];

pub(super) const PATH_PARTS: [&str; 5] = ["fixed", "request", "stored", "parameter", "none"];

/// The injection status of a PHP page whose markup check found a variable
/// and whose settle Choices answer `markup` and `path`, with the path check
/// at `path_check`; and the settle requests sent.
pub(super) fn php_settled(markup: &str, path_check: f64, path: &str) -> (Status, u64) {
    let project = Project::new();
    project.write(
        "index.php",
        "<?php\nrequire_once ROOT . \"parts/{$part}.php\";\n$page['body'] .= \"<div>{$html}</div>\";\necho $page['body'];\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.9)),
        ("markup", noul_at(0.9)),
        ("path", noul_at(path_check)),
        ("origin", spread(0.4, 0.3, 0.3)),
        ("markup_parts", choice_of(markup, &MARKUP_PARTS)),
        ("path_parts", choice_of(path, &PATH_PARTS)),
        ("markup_values", choice_of("raw", &MARKUP_VALUES)),
    ];
    let report = run(&project, &options, &mut eval);
    (
        report.files[0].dimensions[catalog::INJECTION]
            .status
            .clone(),
        report
            .stages
            .get("settle")
            .map_or(0, |stage| stage.successful_requests),
    )
}

pub(super) const SHELL_PARTS: [&str; 6] = [
    "request",
    "stored",
    "parameter",
    "checked",
    "internal",
    "none",
];

/// The injection status and first finding's message of a PHP page with the
/// `nouls` of its trace, the origin at `origin` and `settles` answering its
/// Choices; and the settle requests sent.
pub(super) fn php_page(
    nouls: &[(&'static str, f64)],
    origin: Value,
    settles: Vec<(&'static str, Value)>,
) -> (Status, Option<String>, u64) {
    let project = Project::new();
    project.write(
        "ping.php",
        "<?php\n$ip = $_POST['ip'];\n$parts = explode('.', $ip);\n$out = shell_exec('ping -c 4 ' . $ip);\ninclude PARTS . $part;\necho \"<pre>{$out}</pre>\";\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = scripted(0);
    eval.overrides = standing();
    eval.overrides
        .extend(nouls.iter().map(|(q, p)| (*q, noul_at(*p))));
    eval.overrides.push(("origin", origin));
    eval.overrides.extend(settles);
    let report = run(&project, &options, &mut eval);
    let file = &report.files[0];
    (
        file.dimensions[catalog::INJECTION].status.clone(),
        file.findings.first().map(|f| f.message.clone()),
        report
            .stages
            .get("settle")
            .map_or(0, |stage| stage.successful_requests),
    )
}

#[test]
fn a_php_shell_check_is_settled_by_what_its_command_lines_hold() {
    let found = [("interpreted", 0.95), ("shell", 0.9)];
    let request = spread(0.0, 0.05, 0.95);
    for (held, status) in [
        ("checked", Status::Clear),
        ("internal", Status::Clear),
        ("request", Status::Review),
    ] {
        let settle = vec![("shell_parts", choice_of(held, &SHELL_PARTS))];
        assert_eq!(
            php_page(&found, request.clone(), settle).0,
            status,
            "{held}"
        );
    }
}

#[test]
fn a_php_page_a_leaning_check_made_a_note_is_not_settled() {
    // The markup check leans toward a concern with parameters as the
    // origin: a note, which is not reported, so no Choice is asked.
    let nouls = [("interpreted", 0.9), ("markup", 0.7), ("path", 0.3)];
    let (status, message, settles) = php_page(
        &nouls,
        spread(0.1, 0.8, 0.1),
        vec![
            ("markup_parts", choice_of("built", &MARKUP_PARTS)),
            ("path_parts", choice_of("fixed", &PATH_PARTS)),
        ],
    );
    assert_eq!((status, message, settles), (Status::Clear, None, 0));
}

#[test]
fn a_php_markup_check_is_settled_by_what_is_joined_even_when_it_found_a_variable() {
    assert_eq!(php_settled("built", 0.05, "none"), (Status::Clear, 1));
    for joined in ["escaped", "internal", "data"] {
        assert_eq!(
            php_settled(joined, 0.05, "none").0,
            Status::Clear,
            "{joined}"
        );
    }
    for joined in ["request", "stored"] {
        assert_eq!(
            php_settled(joined, 0.05, "none").0,
            Status::Review,
            "{joined} names another party's value, whatever the origin question said"
        );
    }
    assert_eq!(
        php_settled("parameter", 0.05, "none").0,
        Status::Uncertain,
        "a parameter leaves the found variable with its undecided origin"
    );
    assert_eq!(
        php_settled("built", 0.4, "fixed").0,
        Status::Clear,
        "an undecided include path picked from fixed names"
    );
    for origin in ["request", "stored", "parameter"] {
        assert_eq!(
            php_settled("built", 0.4, origin).0,
            Status::Uncertain,
            "{origin}"
        );
    }
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.9)),
        ("markup", noul_at(0.9)),
        ("origin", spread(0.4, 0.3, 0.3)),
    ];
    let report = run(&project, &options, &mut eval);
    assert!(
        !report.stages.contains_key("settle"),
        "other languages keep a found markup check"
    );
}
