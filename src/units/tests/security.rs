//! Injection, sensitive data and unsafe settings: traces, callers, settle
//! Choices and PHP pages.
use super::*;

const QUERY: &str = "fn find(conn: &Connection, name: &str) -> Result<Row> {\n    let sql = format!(\"SELECT id FROM users WHERE name = '{name}'\");\n    conn.query_row(&sql, [], Row::from)\n}\n\nfn total(a: i32, b: i32) -> i32 {\n    a + b\n}\n";

fn security_project(source: &str) -> (Project, CheckArgs) {
    project_with(&[("lib.rs", source)], &catalog::SECURITY)
}

/// A certain choice of `id` among the two sites of `find` in `QUERY`.
fn site(id: &str) -> Value {
    let probabilities: serde_json::Map<String, Value> = ["S1", "S2", "none"]
        .iter()
        .map(|option| {
            (
                option.to_string(),
                json!(if *option == id { 1.0 } else { 0.0 }),
            )
        })
        .collect();
    json!({"type":"choice","choice":id,"confidence":1.0,"probabilities":probabilities})
}

#[test]
fn only_functions_with_calls_built_text_or_field_assignments_are_sent_for_security() {
    let (project, options) = security_project(QUERY);
    let (_, plan) = planned(&project, &options);
    let sent: Vec<&str> = plan
        .requests
        .iter()
        .flat_map(|p| p.request["state"]["functions"].as_array().unwrap())
        .filter_map(|f| f["name"].as_str())
        .collect();
    assert_eq!(
        sent,
        ["find"],
        "`total` has no call, built text or field assignment"
    );
}

#[test]
fn clear_presence_needs_no_trace_and_clears_every_security_rule() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages, ["first"]);
    for rule in catalog::SECURITY {
        assert_eq!(report.files[0].dimensions[rule].status, Status::Clear);
    }
}

#[test]
fn an_unhandled_value_from_another_party_is_a_located_injection_review() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.95)),
        ("sql", noul_at(0.95)),
        ("origin", spread(0.0, 0.1, 0.9)),
        ("site", site("S1")),
    ];
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages, ["first", "first"], "one trace, no recheck");
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.rule, "security/injection");
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(finding.category.as_deref(), Some("CWE-89 SQL injection"));
    assert_eq!(finding.line, 2, "located at the chosen site");
    assert!(finding.action.contains("bound query parameters"));
}

#[test]
fn a_parameter_origin_is_a_consider_that_callers_can_settle() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection) -> Result<Row> {{\n    find(conn, \"admin\")\n}}\n"
    );
    let overrides = || {
        vec![
            ("interpreted", noul_at(0.95)),
            ("sql", noul_at(0.95)),
            ("origin", spread(0.0, 0.9, 0.1)),
        ]
    };
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = overrides();
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        report.files[0].dimensions[catalog::INJECTION].status,
        Status::Consider,
        "no caller is known, so the parameter stays a concern"
    );
    let (project, options) = security_project(&caller);
    let mut eval = scripted(0);
    eval.overrides = overrides();
    eval.recheck_level = Some(0);
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages.last().unwrap(), "recheck");
    let find = |report: &Report| {
        report.files[0]
            .findings
            .iter()
            .any(|f| f.rule == "security/injection" && f.symbol.as_deref() == Some("find"))
    };
    assert!(!find(&report), "the caller passes a fixed value");
}

#[test]
fn a_check_left_undecided_is_decided_again_with_callers() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection, request: &Request) -> Result<Row> {{\n    find(conn, &request.query[\"name\"])\n}}\n"
    );
    let (project, options) = security_project(&caller);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("path", noul_at(0.5)),
        ("origin", spread(0.0, 0.9, 0.1)),
    ];
    eval.recheck_level = Some(2);
    let report = run(&project, &options, &mut eval);
    let finding = report.files[0]
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("find") && f.rule == "security/injection")
        .expect("the recheck decides the path check and the origin");
    assert_eq!(
        finding.strength,
        Strength::Review,
        "undecided without the recheck"
    );
    assert!(finding.category.is_some());
}

#[test]
fn a_parameter_in_a_path_or_url_is_a_note_until_callers_show_another_party() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("url", noul_at(0.95)),
        ("origin", spread(0.0, 0.9, 0.1)),
    ];
    let finding = &first_finding(&project, &options, &mut eval);
    assert_eq!(finding.strength, Strength::Note);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-918 server-side request forgery")
    );
}

const FETCH_QUOTE: &str = "fn quote(client: &Client, base: &Url, symbol: &str) -> String {\n    let url = base.join(&format!(\"quotes/{symbol}\")).unwrap();\n    client.get(url).send().unwrap().text().unwrap()\n}\n";

const URL_PARTS: [&str; 5] = ["own", "forwards", "given", "outside", "none"];

const RUNS_IN: [&str; 3] = ["browser", "server", "either"];

/// Injection status and settle requests with the URL (or path) check at
/// `check` undecided and the settle Choice answering `parts`.
fn settled_injection(
    project: &Project,
    options: &CheckArgs,
    check: &'static str,
    parts: &str,
) -> (Status, u64) {
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        (check, noul_at(0.4)),
        ("origin", spread(0.0, 0.9, 0.1)),
        ("url_parts", choice_of(parts, &URL_PARTS)),
        ("runs_in", choice_of("server", &RUNS_IN)),
    ];
    let report = run(project, options, &mut eval);
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

#[test]
fn an_undecided_url_is_settled_only_by_a_host_of_the_programs_own() {
    let (project, mut options) = security_project(FETCH_QUOTE);
    assert_eq!(
        settled_injection(&project, &options, "url", "own"),
        (Status::Clear, 2),
        "where its URLs come from and where it runs"
    );
    for parts in ["forwards", "given", "outside"] {
        options.refresh = true;
        assert_eq!(
            settled_injection(&project, &options, "url", parts).0,
            Status::Uncertain,
            "{parts}"
        );
    }
    options.refresh = true;
    assert_eq!(
        settled_injection(&project, &options, "path", "own"),
        (Status::Uncertain, 0),
        "an undecided path is not settled"
    );
}

#[test]
fn error_details_are_settled_by_where_the_text_goes() {
    let (project, mut options) = security_project(QUERY);
    // `own` 0.1 names a foreign message, which with a leaning exception would
    // otherwise raise a consider.
    let status = |destination: &str, (exception, own): (f64, f64), options: &CheckArgs| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("error_details", noul_at(0.3)),
            ("exception_to_client", noul_at(exception)),
            ("own_messages", noul_at(own)),
            (
                "destination",
                choice_of(
                    destination,
                    &["client", "local", "logs", "caller", "stored"],
                ),
            ),
        ];
        run(&project, options, &mut eval).files[0].dimensions[catalog::SENSITIVE_DATA]
            .status
            .clone()
    };
    assert_eq!(status("local", (0.3, 0.5), &options), Status::Clear);
    options.refresh = true;
    assert_eq!(status("client", (0.3, 0.5), &options), Status::Uncertain);
    assert_eq!(
        status("local", (0.6, 0.1), &options),
        Status::Clear,
        "a foreign message that stays on the local terminal"
    );
}

#[test]
fn checks_that_all_clear_rule_out_an_uncertain_presence() {
    let (project, options) = security_project(QUERY);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.5)),
        ("origin", spread(0.0, 1.0, 0.0)),
    ];
    let report = run(&project, &options, &mut eval);
    assert_eq!(
        report.files[0].dimensions[catalog::INJECTION].status,
        Status::Clear
    );
}

#[test]
fn development_only_exposure_is_one_level_lower_and_names_its_weakness() {
    let (project, options) = security_project(QUERY);
    let report = run_with_nouls(
        &project,
        &options,
        &[("logs_secret", 0.95), ("dev_only", 0.95)],
    );
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.rule, "security/sensitive-data");
    assert_eq!(finding.strength, Strength::Consider);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-532 sensitive data in logs")
    );
    assert!(finding.message.contains("runs only in development"));
}

#[test]
fn top_level_setup_is_one_unit_for_unsafe_settings() {
    let (project, options) = project_with(
        &[(
            "server.ts",
            "const app = express()\napp.use(cors({ origin: true, credentials: true }))\n",
        )],
        &[catalog::UNSAFE_SETTINGS],
    );
    let (_, plan) = planned(&project, &options);
    let request = &plan.requests[0].request;
    assert!(
        request["state"]["module"]["source"]
            .as_str()
            .unwrap()
            .contains("app.use(cors(")
    );
    let report = run_with_nouls(&project, &options, &[("weakened", 0.95), ("cors", 0.95)]);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.category.as_deref(), Some("CWE-942 permissive CORS"));
    assert!(finding.message.starts_with("Module setup"));
}

const PAGE: &str = "<?php\nrequire_once 'lib.php';\n\nfunction escape_html($text) {\n    return htmlspecialchars($text, ENT_QUOTES);\n}\n\nif (isset($_GET['id'])) {\n    $id = $_GET['id'];\n    $query = \"SELECT name FROM users WHERE id = '$id'\";\n    $result = mysqli_query($db, $query);\n    echo '<p>' . $_GET['name'] . '</p>';\n}\n?>\n<footer><?= date('Y') ?></footer>\n";

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
fn redirects_deserializers_and_uploads_are_checked_kinds_with_their_weakness() {
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    for (check, category) in [
        ("redirect", "CWE-601 open redirect"),
        ("deserialize", "CWE-502 deserialization of untrusted data"),
        ("upload", "CWE-434 unrestricted file upload"),
    ] {
        let project = Project::new();
        project.write(
            "go.php",
            "<?php\n$target = $_GET['next'];\nheader('Location: ' . $target);\n$prefs = unserialize($_COOKIE['prefs']);\nmove_uploaded_file($_FILES['f']['tmp_name'], 'up/' . $_FILES['f']['name']);\n",
        );
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("resource", noul_at(0.95)),
            (check, noul_at(0.95)),
            ("origin", spread(0.0, 0.05, 0.95)),
        ];
        let report = run(&project, &options, &mut eval);
        let finding = &report.files[0].findings[0];
        assert_eq!(finding.category.as_deref(), Some(category));
        assert_eq!(finding.strength, Strength::Review);
    }
}

/// The injection plan of a project holding only `source` at `path`.
fn injection_plan(path: &str, source: &str) -> Plan {
    let project = Project::new();
    project.write(path, source);
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    planned(&project, &options).1
}

/// The questions of the trace planned for the first unit of `path`.
fn traced_checks(path: &str, source: &str) -> serde_json::Map<String, Value> {
    let plan = injection_plan(path, source);
    let trace = &plan.files[&0].units[0];
    let Detail::Security {
        trace: Some((request, _)),
        ..
    } = &trace.detail
    else {
        panic!("no trace planned");
    };
    request["questions"].as_object().unwrap().clone()
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

const PICKLED: &str = "import pickle\n\nfrom flask import jsonify, request\n\n\ndef restore_cart():\n    cart = pickle.loads(request.get_data())\n    return jsonify(items=len(cart))\n";

#[test]
fn a_deserializer_is_asked_about_only_where_the_source_names_one() {
    let first = |path: &str, source: &str| {
        injection_plan(path, source).requests[0].request["questions"]["f0_interpreted"].clone()
    };
    let named = first("shop/cart.py", PICKLED);
    assert!(named.to_string().contains("pickle, marshal"), "{named}");
    let parsed = PICKLED
        .replace("import pickle", "import json")
        .replace("pickle.loads", "json.loads");
    assert_eq!(
        first("shop/cart.py", &parsed),
        questions::security_interpreted("functions[0].source", false, None, false),
        "code that names no deserializer keeps its question and cached answer"
    );
    assert!(traced_checks("shop/cart.py", PICKLED).contains_key("deserialize"));
    assert!(!traced_checks("shop/cart.py", &parsed).contains_key("deserialize"));
    let ruby = traced_checks(
        "app/models/cart.rb",
        "class Cart\n  def self.restore(params)\n    Marshal.load(Base64.decode64(params[:cart]))\n  end\nend\n",
    );
    assert!(ruby["deserialize"].to_string().contains("Marshal.load"));
    let java = traced_checks(
        "src/main/java/shop/Cart.java",
        "class Cart {\n  Object restore(InputStream body) throws Exception {\n    return new ObjectInputStream(body).readObject();\n  }\n}\n",
    );
    assert!(
        java["deserialize"]
            .to_string()
            .contains("ObjectInputStream")
    );
}

#[test]
fn request_data_given_to_pickle_is_a_deserialization_review() {
    let project = Project::new();
    project.write("shop/cart.py", PICKLED);
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.95)),
        ("deserialize", noul_at(0.95)),
        ("origin", spread(0.0, 0.05, 0.95)),
    ];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-502 deserialization of untrusted data")
    );
    assert_eq!(finding.strength, Strength::Review);
}

const XML_IMPORT: &str = "import java.io.InputStream;\nimport javax.xml.parsers.DocumentBuilderFactory;\n\nclass Catalog {\n  int count(InputStream body) throws Exception {\n    var document = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(body);\n    return document.getElementsByTagName(\"item\").getLength();\n  }\n}\n";

#[test]
fn xml_parsed_with_entities_is_asked_about_only_where_a_parser_is_named() {
    let path = "src/main/java/shop/Catalog.java";
    let named = traced_checks(path, XML_IMPORT);
    assert!(named["xxe"].to_string().contains("DocumentBuilderFactory"));
    let plain = XML_IMPORT
        .replace("import javax.xml.parsers.DocumentBuilderFactory;\n", "")
        .replace(
            "DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(body)",
            "Json.parse(body)",
        );
    assert!(!traced_checks(path, &plain).contains_key("xxe"));
    // A module that imports the parser at its top, as Python code does.
    let module = "from lxml import etree\n\n\ndef count(body):\n    root = etree.fromstring(body)\n    return len(root.findall('item'))\n";
    assert!(traced_checks("shop/catalog.py", module).contains_key("xxe"));
    let project = Project::new();
    project.write(path, XML_IMPORT);
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.95)),
        ("xxe", noul_at(0.95)),
        ("origin", spread(0.0, 0.05, 0.95)),
    ];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-611 XML external entity reference")
    );
    assert_eq!(finding.strength, Strength::Review);
}

const MARKUP_PARTS: [&str; 8] = [
    "request",
    "stored",
    "parameter",
    "escaped",
    "internal",
    "built",
    "data",
    "none",
];

const PATH_PARTS: [&str; 5] = ["fixed", "request", "stored", "parameter", "none"];

/// The injection status of a PHP page whose markup check found a variable
/// and whose settle Choices answer `markup` and `path`, with the path check
/// at `path_check`; and the settle requests sent.
fn php_settled(markup: &str, path_check: f64, path: &str) -> (Status, u64) {
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

const SHELL_PARTS: [&str; 6] = [
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
fn php_page(
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
    eval.overrides = nouls.iter().map(|(q, p)| (*q, noul_at(*p))).collect();
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
fn a_php_path_choice_is_asked_when_a_leaning_check_made_the_page_a_note() {
    // The markup check leans toward a concern with parameters as the
    // origin, a note; once its Choice clears it, the undecided path check
    // is left, and its own Choice is asked in the same settle round.
    let nouls = [("interpreted", 0.9), ("markup", 0.7), ("path", 0.3)];
    let parameters = spread(0.1, 0.8, 0.1);
    let (status, _, settles) = php_page(
        &nouls,
        parameters.clone(),
        vec![
            ("markup_parts", choice_of("built", &MARKUP_PARTS)),
            ("path_parts", choice_of("fixed", &PATH_PARTS)),
        ],
    );
    assert_eq!((status, settles), (Status::Clear, 2));
    let (status, message, _) = php_page(
        &nouls,
        parameters,
        vec![
            ("markup_parts", choice_of("parameter", &MARKUP_PARTS)),
            ("path_parts", choice_of("fixed", &PATH_PARTS)),
        ],
    );
    assert_eq!(status, Status::Note);
    let message = message.unwrap();
    assert!(
        message
            .starts_with("Top-level code places a value whose origin it does not show into markup"),
        "a page script has no parameters: {message}"
    );
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

#[test]
fn an_undecided_value_is_cleared_by_its_kind_or_leans_into_a_note() {
    let (project, mut options) = hardcoded_project();
    let split = || {
        vec![
            ("environment", spread(0.4, 0.1, 0.5)),
            ("magic", spread(0.6, 0.1, 0.3)),
        ]
    };
    let mut eval = scripted(0);
    eval.overrides = split();
    let report = run(&project, &options, &mut eval);
    assert!(eval.stages.contains(&"recheck".to_string()));
    let dimension = &report.files[0].dimensions["hardcoded_values"];
    assert_eq!(
        (dimension.units.note, dimension.units.uncertain),
        (2, 0),
        "not cleared by the kind checks, and leaning toward the concern"
    );
    let leaned = report.files[0]
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("connect"))
        .expect("the environment answer leaned toward its concern");
    assert_eq!(leaned.strength, Strength::Note);
    assert!(
        leaned.message.contains("the answer was split"),
        "{}",
        leaned.message
    );
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides = split();
    eval.recheck_level = Some(2);
    let report = run(&project, &options, &mut eval);
    let dimension = &report.files[0].dimensions["hardcoded_values"];
    assert_eq!(
        dimension.status,
        Status::Clear,
        "every value is of an acceptable kind"
    );
}

#[test]
fn error_details_clear_on_the_programs_own_messages_or_lean_into_a_note() {
    let (project, mut options) = security_project(QUERY);
    let status = |report: &Report| {
        report.files[0].dimensions[catalog::SENSITIVE_DATA]
            .status
            .clone()
    };
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("error_details", noul_at(0.3)),
        ("exception_to_client", noul_at(0.3)),
        ("own_messages", noul_at(0.95)),
    ];
    assert_eq!(status(&run(&project, &options, &mut eval)), Status::Clear);
    options.refresh = true;
    let report = run_with_nouls(
        &project,
        &options,
        &[
            ("error_details", 0.6),
            ("exception_to_client", 0.3),
            ("own_messages", 0.5),
        ],
    );
    let note = &report.files[0].findings[0];
    assert_eq!(note.strength, Strength::Note);
    assert!(
        note.message.contains("may send internal error details"),
        "{}",
        note.message
    );
    assert_eq!(
        note.category.as_deref(),
        Some("CWE-209 error details exposed")
    );
}

#[test]
fn a_foreign_error_message_names_the_error_text_in_an_error_detail_note() {
    let (project, options) = security_project(QUERY);
    let report = run_with_nouls(
        &project,
        &options,
        &[
            ("error_details", 0.3),
            ("exception_to_client", 0.6),
            ("own_messages", 0.1),
        ],
    );
    let finding = &report.files[0].findings[0];
    assert_eq!(
        finding.strength,
        Strength::Note,
        "where the text goes is still undecided"
    );
    assert!(
        finding
            .message
            .contains("text of a library or database error"),
        "{}",
        finding.message
    );
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-209 error details exposed")
    );
}

#[test]
fn an_undecided_caller_recheck_replaces_an_undecided_traced_lean() {
    let caller = format!(
        "{QUERY}\nfn handler(conn: &Connection, dir: &Path) -> Result<Row> {{\n    find(conn, &dir.join(\"cache\"))\n}}\n"
    );
    let (project, options) = security_project(&caller);
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("path", noul_at(0.6)),
        ("origin", spread(0.0, 0.9, 0.1)),
    ];
    eval.recheck_level = Some(0);
    eval.recheck_overrides = vec![("path", noul_at(0.3)), ("origin", spread(0.1, 0.9, 0.0))];
    let report = run(&project, &options, &mut eval);
    assert_eq!(eval.stages.last().unwrap(), "recheck");
    let file = &report.files[0];
    assert!(
        !file
            .findings
            .iter()
            .any(|f| f.rule == "security/injection" && f.symbol.as_deref() == Some("find")),
        "the callers' answer leans away, so no note"
    );
}

const SERVICE: &str = "def find_asset(asset_id):\n    asset = ASSETS.get(asset_id)\n    if asset is None:\n        raise LookupError(\"Asset not found\")\n    return asset\n";
const HANDLER: &str = "from fastapi import HTTPException\n\nfrom app.services import find_asset\n\n\ndef read_asset(asset_id: str):\n    try:\n        return find_asset(asset_id)\n    except LookupError as exc:\n        raise HTTPException(status_code=404, detail=str(exc)) from exc\n";

#[test]
fn an_error_trace_shows_the_errors_the_called_functions_raise() {
    let (project, options) = project_with(
        &[("app/services.py", SERVICE), ("app/api.py", HANDLER)],
        &[catalog::SENSITIVE_DATA],
    );
    let (inputs, plan) = planned(&project, &options);
    let owner = inputs
        .iter()
        .position(|i| i.result.path.ends_with("api.py"))
        .unwrap();
    let Detail::Security {
        trace: Some((trace, _)),
        ..
    } = &plan.files[&owner].units[0].detail
    else {
        panic!("a traced security unit");
    };
    assert_eq!(
        trace["state"]["errors_created_by_functions_it_calls"],
        json!([{"function": "find_asset", "error": "LookupError", "message": "\"Asset not found\""}])
    );
    assert_eq!(
        trace["questions"]["exception_to_client"],
        questions::EXCEPTION_TO_CLIENT_FROM_CALLEES.body("function.source")
    );
    let names_callees = |trace: &Value| {
        trace["questions"]["messages"]
            .to_string()
            .contains("errors_created_by_functions_it_calls")
    };
    assert!(
        names_callees(trace),
        "passing on a callee's own error text is the program's own"
    );
    let alone = project_with(&[("app/api.py", HANDLER)], &[catalog::SENSITIVE_DATA]);
    let (_, plan) = planned(&alone.0, &alone.1);
    let Detail::Security {
        trace: Some((trace, _)),
        ..
    } = &plan.files[&0].units[0].detail
    else {
        panic!("a traced security unit");
    };
    assert!(
        trace["state"]
            .get("errors_created_by_functions_it_calls")
            .is_none()
    );
    assert!(
        !trace["questions"]["exception_to_client"]
            .to_string()
            .contains("errors_created_by_functions_it_calls"),
        "without callee errors the check is asked as before"
    );
    assert!(trace["questions"].get("messages").is_some() && !names_callees(trace));
}

const ROUTE: &str = "export async function loadThing(c: Context) {\n  const { data, error } = await db.from('things').select('*').eq('id', c.req.param('id'))\n  if (error) throw new InternalError(`Query failed: ${error.message}`, error)\n  if (!data) throw new NotFoundError('Thing not found')\n  return c.json(data)\n}\n";

#[test]
fn each_created_error_message_is_asked_about_and_names_the_foreign_one() {
    let project = Project::new();
    project.write("routes.ts", ROUTE);
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    let Detail::Security {
        trace: Some((trace, _)),
        messages,
        ..
    } = &plan.files[&0].units[0].detail
    else {
        panic!("a traced security unit");
    };
    assert_eq!(
        messages,
        &["`Query failed: ${error.message}`", "'Thing not found'"]
    );
    assert_eq!(trace["state"]["messages"][1]["id"], "m1");
    assert!(trace["questions"].get("messages").is_some());
    assert!(
        trace["questions"].get("own_messages").is_none(),
        "the Choice replaces the Noul"
    );
    let run_with = |options: &CheckArgs, chosen: &str| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("error_details", noul_at(0.3)),
            ("exception_to_client", noul_at(0.6)),
            ("messages", choice_of(chosen, &["m0", "m1", "none"])),
            to_client(),
        ];
        run(&project, options, &mut eval)
    };
    let report = run_with(&options, "none");
    assert_eq!(
        report.files[0].dimensions[catalog::SENSITIVE_DATA].status,
        Status::Clear,
        "every message is the program's own"
    );
    options.refresh = true;
    let report = run_with(&options, "m0");
    let finding = &report.files[0].findings[0];
    assert_eq!(
        finding.strength,
        Strength::Note,
        "where the text goes is still undecided"
    );
    assert!(
        finding
            .message
            .ends_with("into an error message, which may reach a remote client."),
        "{}",
        finding.message
    );
}

#[test]
fn an_injection_trace_shows_the_enums_its_sites_name() {
    let project = Project::new();
    project.write(
        "server/store.ts",
        "import { ConfigKey } from '../shared/config'\n\nexport async function setAITagConfig(DB: D1Database, config: AITagConfig): Promise<boolean> {\n  const insertSql = `INSERT INTO stores (key, value) VALUES ('${ConfigKey.aiTag}', ?) ON CONFLICT(key) DO UPDATE SET value = ?`\n  const bindValue = JSON.stringify(config)\n  const result = await DB.prepare(insertSql).bind(bindValue, bindValue).run()\n  return result.success\n}\n",
    );
    project.write(
        "shared/config.ts",
        "enum ConfigKey {\n  shouldShowRecent = 'config/should_show_recent',\n  aiTag = 'config/ai_tag',\n}\n\nexport { ConfigKey }\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let (_, plan) = planned(&project, &options);
    let traces: Vec<&Value> = plan
        .files
        .values()
        .flat_map(|f| &f.units)
        .filter_map(|u| match &u.detail {
            Detail::Security {
                trace: Some((request, _)),
                ..
            } => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(traces.len(), 1);
    assert!(
        traces[0]["state"]["enums_named_in_sites"][0]
            .as_str()
            .is_some_and(|e| e.starts_with("enum ConfigKey {")),
        "{}",
        traces[0]["state"]
    );
}

#[test]
fn a_broad_weak_setting_answer_that_no_check_names_is_a_note() {
    let (project, options) = security_project(QUERY);
    let unnamed = run_with_nouls(&project, &options, &[("weakened", 0.95), ("debug", 0.95)]);
    let finding = &unnamed.files[0].findings[0];
    assert_eq!(finding.rule, "security/unsafe-settings");
    assert_eq!(finding.strength, Strength::Note);
    let asked = |report: &Report, question: &str| {
        report.files[0]
            .judgments
            .iter()
            .any(|j| j.question == question)
    };
    assert!(
        asked(&unnamed, "tls") && !asked(&unnamed, "debug") && !asked(&unnamed, "type"),
        "the C# checks are asked only about C# files"
    );
    let (project, options) = security_project(QUERY);
    let named = run_with_nouls(&project, &options, &[("weakened", 0.95), ("cookie", 0.95)]);
    assert_eq!(named.files[0].findings[0].strength, Strength::Review);

    let project = Project::new();
    project.write(
        "Program.cs",
        "var app = WebApplication.CreateBuilder(args).Build();\napp.UseDeveloperExceptionPage();\napp.MapGet(\"/\", () => \"Hello\");\napp.Run();\n",
    );
    let mut options = args();
    options.rules = vec![catalog::UNSAFE_SETTINGS.into()];
    let named = run_with_nouls(&project, &options, &[("weakened", 0.95), ("debug", 0.95)]);
    assert!(asked(&named, "debug") && asked(&named, "token"));
    let finding = &named.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-489 active debug code")
    );
    assert!(
        finding.message.contains("detailed error pages"),
        "{}",
        finding.message
    );
}

/// Answers like `Scripted` and keeps every request it was sent.
struct Recording {
    inner: Scripted,
    requests: Vec<Value>,
}

impl crate::transport::Evaluator for Recording {
    fn evaluate(&mut self, request: &Value) -> anyhow::Result<Value> {
        self.requests.push(request.clone());
        self.inner.evaluate(request)
    }
}

fn recording(nouls: &[(&'static str, f64)]) -> Recording {
    let mut inner = scripted(0);
    inner.overrides = nouls.iter().map(|&(q, p)| (q, noul_at(p))).collect();
    Recording {
        inner,
        requests: Vec::new(),
    }
}

#[test]
fn a_csharp_setup_trace_shows_the_constants_it_names_and_finds_a_key_written_in_code() {
    let project = Project::new();
    project.write(
        "src/Api/AuthorizationConstants.cs",
        "namespace Api;\n\npublic class AuthorizationConstants\n{\n    public const string JWT_SECRET_KEY = \"SecretKeyOfDoomThatMustBeLong\";\n    public const int PAGE_SIZE = 50;\n}\n",
    );
    project.write(
        "src/Api/Program.cs",
        "var builder = WebApplication.CreateBuilder(args);\nvar key = Encoding.ASCII.GetBytes(AuthorizationConstants.JWT_SECRET_KEY);\nbuilder.Services.AddAuthentication().AddJwtBearer(o => o.TokenValidationParameters = new TokenValidationParameters { IssuerSigningKey = new SymmetricSecurityKey(key) });\nvar app = builder.Build();\napp.Run();\n",
    );
    let mut options = args();
    options.rules = vec![catalog::UNSAFE_SETTINGS.into()];
    let mut eval = recording(&[("weakened", 0.95), ("key", 0.95)]);
    let report = run(&project, &options, &mut eval);
    let trace = eval
        .requests
        .iter()
        .find(|r| r["jevgate"]["stage"] == "trace")
        .unwrap();
    assert_eq!(
        trace["state"]["constants_named"],
        json!(["AuthorizationConstants.JWT_SECRET_KEY = \"SecretKeyOfDoomThatMustBeLong\""]),
        "only the constants the setup names"
    );
    assert!(trace["questions"]["key"].is_object() && trace["questions"]["debug"].is_object());
    let program = report
        .files
        .iter()
        .find(|f| f.path.ends_with("Program.cs"))
        .unwrap();
    let finding = &program.findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-321 hard-coded cryptographic key")
    );
    assert!(
        finding.message.contains("key written in the code"),
        "{}",
        finding.message
    );
}

/// An unsafe-settings run over one file, with `nouls` answered and an
/// optional settle Choice: its report and the requests sent.
fn settings_run(
    path: &str,
    source: &str,
    nouls: &[(&'static str, f64)],
    settle: Option<(&'static str, Value)>,
) -> (Report, Vec<Value>) {
    let project = Project::new();
    project.write(path, source);
    let mut options = args();
    options.rules = vec![catalog::UNSAFE_SETTINGS.into()];
    let mut eval = recording(nouls);
    eval.inner.overrides.extend(settle);
    let report = run(&project, &options, &mut eval);
    (report, eval.requests)
}

/// The questions of the trace among `requests`.
fn trace_questions(requests: &[Value]) -> serde_json::Map<String, Value> {
    requests
        .iter()
        .find(|r| r["jevgate"]["stage"] == "trace")
        .unwrap()["questions"]
        .as_object()
        .unwrap()
        .clone()
}

#[test]
fn code_outside_csharp_and_django_is_asked_about_tokens_keys_and_escaping() {
    let (report, requests) = settings_run(
        "server.js",
        "const session = require('express-session');\nconst app = require('express')();\napp.use(session({ secret: 'keyboard cat', resave: true, saveUninitialized: true }));\napp.listen(9090);\n",
        &[("weakened", 0.95), ("key", 0.95)],
        None,
    );
    let questions = trace_questions(&requests);
    for check in ["token", "key", "escape", "hash", "cookie"] {
        assert!(questions[check].is_object(), "{check}");
    }
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-321 hard-coded cryptographic key")
    );
    // C# asks its own wording of the token check, once.
    let (_, requests) = settings_run(
        "Program.cs",
        "var builder = WebApplication.CreateBuilder(args);\nbuilder.Services.AddCors(o => o.AddDefaultPolicy(p => p.AllowAnyOrigin()));\nvar app = builder.Build();\napp.Run();\n",
        &[("weakened", 0.95)],
        None,
    );
    assert!(
        trace_questions(&requests)["token"]
            .to_string()
            .contains("ValidateIssuerSigningKey")
    );
}

#[test]
fn a_token_the_code_only_passes_on_is_no_review() {
    const USES: [&str; 7] = [
        "verifies",
        "passes",
        "verified_before",
        "reads_claims",
        "decides_access",
        "turned_off",
        "none",
    ];
    let strength = |choice: &str| {
        let (report, requests) = settings_run(
            "src/useAuth.ts",
            "export function useAuth() {\n  const token = localStorage.getItem('access_token');\n  return fetch('/api/me', { headers: { Authorization: `Bearer ${token}` } });\n}\n",
            &[("weakened", 0.95), ("token", 0.9)],
            Some(("token_use", choice_of(choice, &USES))),
        );
        assert!(
            requests
                .iter()
                .any(|r| r["questions"]["token_use"].is_object()),
            "asked although the check found a concern"
        );
        report.files[0].findings.first().map(|f| f.strength)
    };
    assert_eq!(strength("turned_off"), Some(Strength::Review));
    assert_eq!(
        strength("decides_access"),
        Some(Strength::Consider),
        "whether a token was verified before lies outside the function"
    );
    assert_eq!(strength("reads_claims"), Some(Strength::Note));
    assert_eq!(
        strength("passes"),
        Some(Strength::Note),
        "the broad answer alone names no setting"
    );
}

#[test]
fn a_password_saved_as_plain_text_is_a_consider_and_one_hashed_fast_a_review() {
    const HANDLING: [&str; 4] = ["slow_hash", "plain", "fast_hash", "none"];
    let strength = |choice: &str| {
        let (report, _) = settings_run(
            "src/users.ts",
            "export async function register(repo, name, password) {\n  const user = repo.create({ name, password });\n  await repo.save(user);\n  return user;\n}\n",
            &[("weakened", 0.95), ("hash", 0.9)],
            Some(("password_handling", choice_of(choice, &HANDLING))),
        );
        report.files[0].findings.first().map(|f| f.strength)
    };
    assert_eq!(strength("fast_hash"), Some(Strength::Review));
    assert_eq!(
        strength("plain"),
        Some(Strength::Consider),
        "a callee or model hook may hash what the function saves"
    );
}

#[test]
fn a_csharp_type_named_by_input_is_an_injection_named_by_its_own_check() {
    let project = Project::new();
    project.write(
        "Controllers/ImportsController.cs",
        "namespace Api;\n\npublic class ImportsController : Controller\n{\n    [HttpPost]\n    public IActionResult Post(string typeName, string xml)\n    {\n        var serializer = new XmlSerializer(Type.GetType(typeName));\n        return Ok(serializer.Deserialize(new StringReader(xml)));\n    }\n}\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = recording(&[("interpreted", 0.95), ("type", 0.95)]);
    eval.inner
        .overrides
        .push(("origin", spread(0.0, 0.05, 0.95)));
    let report = run(&project, &options, &mut eval);
    let first = &eval.requests[0]["questions"]["f0_interpreted"];
    assert!(
        first["instructions"]["question"]
            .as_str()
            .unwrap()
            .ends_with("or into the type of objects it creates?"),
        "{first}"
    );
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-502 deserialization of untrusted data")
    );
    assert!(
        finding.message.contains("types of objects it creates"),
        "{}",
        finding.message
    );
}

/// The status of `rule` and the number of settle requests, with `nouls`
/// leaving one check undecided and `settle` answering its Choice.
fn settled_status(
    project: &Project,
    options: &CheckArgs,
    rule: &str,
    nouls: &[(&'static str, f64)],
    settle: (&'static str, Value),
) -> (Status, u64) {
    let mut eval = scripted(0);
    eval.overrides = nouls.iter().map(|&(q, p)| (q, noul_at(p))).collect();
    eval.overrides.push(("origin", spread(0.0, 0.9, 0.1)));
    eval.overrides.push(settle);
    let report = run(project, options, &mut eval);
    (
        report.files[0].dimensions[rule].status.clone(),
        report
            .stages
            .get("settle")
            .map_or(0, |stage| stage.successful_requests),
    )
}

#[test]
fn undecided_markup_cors_cookies_and_logged_objects_are_settled_by_their_choices() {
    let (project, mut options) = security_project(QUERY);
    let markup = ["escaped", "text", "raw", "none"];
    let undecided_markup = [("interpreted", 0.95), ("markup", 0.4)];
    for (chosen, status) in [("escaped", Status::Clear), ("raw", Status::Uncertain)] {
        options.refresh = true;
        let settle = ("markup_output", choice_of(chosen, &markup));
        assert_eq!(
            settled_status(
                &project,
                &options,
                catalog::INJECTION,
                &undecided_markup,
                settle
            ),
            (status, 1),
            "{chosen}"
        );
    }
    let origins = ["unset", "listed", "public", "any"];
    let undecided_cors = [("weakened", 0.4), ("cors", 0.3)];
    for (chosen, status) in [("public", Status::Clear), ("any", Status::Uncertain)] {
        options.refresh = true;
        let settle = ("cors_origins", choice_of(chosen, &origins));
        assert_eq!(
            settled_status(
                &project,
                &options,
                catalog::UNSAFE_SETTINGS,
                &undecided_cors,
                settle
            )
            .0,
            status,
            "{chosen}"
        );
    }
    let cookies = ["unset", "flagged", "missing"];
    let undecided_cookie = [("weakened", 0.4), ("cookie", 0.3)];
    for (chosen, status) in [("flagged", Status::Clear), ("missing", Status::Uncertain)] {
        options.refresh = true;
        let settle = ("cookie_flags", choice_of(chosen, &cookies));
        assert_eq!(
            settled_status(
                &project,
                &options,
                catalog::UNSAFE_SETTINGS,
                &undecided_cookie,
                settle
            )
            .0,
            status,
            "{chosen}"
        );
    }
    let logs = [
        "plain", "identity", "operator", "secret", "personal", "none",
    ];
    let undecided_logs = [("logs_secret", 0.4), ("logs_object_secret", 0.4)];
    for (chosen, status) in [("plain", Status::Clear), ("secret", Status::Uncertain)] {
        options.refresh = true;
        let settle = ("logged", choice_of(chosen, &logs));
        assert_eq!(
            settled_status(
                &project,
                &options,
                catalog::SENSITIVE_DATA,
                &undecided_logs,
                settle
            )
            .0,
            status,
            "{chosen}"
        );
    }
}

#[test]
fn an_audit_line_naming_who_signed_in_is_no_logged_personal_data() {
    let (project, mut options) = security_project(QUERY);
    let logs = [
        "plain", "identity", "operator", "secret", "personal", "none",
    ];
    let found = [("logs_secret", 0.92)];
    for (chosen, status) in [("identity", Status::Clear), ("secret", Status::Review)] {
        options.refresh = true;
        let settle = ("logged", choice_of(chosen, &logs));
        let (outcome, settles) =
            settled_status(&project, &options, catalog::SENSITIVE_DATA, &found, settle);
        assert_eq!(
            settles, 1,
            "asked although the presence question found a concern"
        );
        assert_eq!(outcome, status, "{chosen}");
    }
}

#[test]
fn a_decided_check_asks_no_settle_choice() {
    let (project, options) = security_project(QUERY);
    let settle = ("markup_output", choice_of("escaped", &["escaped", "raw"]));
    let (status, settles) = settled_status(
        &project,
        &options,
        catalog::INJECTION,
        &[("interpreted", 0.95), ("markup", 0.95)],
        settle,
    );
    assert_eq!(settles, 0, "a markup check at review is not second-guessed");
    assert_eq!(status, Status::Consider);
}

#[test]
fn a_function_added_to_one_run_is_the_only_security_request_asked_again() {
    // Runs end after `f2`, `f4` and `f8`, whose names hash to an end.
    let query = |name: &str| {
        format!(
            "fn {name}(conn: &Connection, name: &str) -> Result<Row> {{\n    let sql = format!(\"SELECT id FROM users WHERE name = '{{name}}'\");\n    conn.query_row(&sql, [], Row::from)\n}}\n\n"
        )
    };
    let source = |added: bool| -> String {
        (0..14)
            .map(|i| match i {
                3 if added => format!("{}{}", query("f3"), query("g")),
                _ => query(&format!("f{i}")),
            })
            .collect()
    };
    let (sizes, before) = packs(
        &[("lib.rs", &source(false))],
        &catalog::SECURITY,
        "security",
    );
    assert_eq!(sizes, [3, 2, 4, 5]);
    let (sizes, after) = packs(&[("lib.rs", &source(true))], &catalog::SECURITY, "security");
    assert_eq!(sizes, [3, 3, 4, 5]);
    only_changed(&before, &after, 1);
}
