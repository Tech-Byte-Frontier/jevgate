//! Injection, sensitive data and unsafe settings: traces, callers and error handlers.
use super::*;

const QUERY: &str = "fn find(conn: &Connection, name: &str) -> Result<Row> {\n    let sql = format!(\"SELECT id FROM users WHERE name = '{name}'\");\n    conn.query_row(&sql, [], Row::from)\n}\n\nfn total(a: i32, b: i32) -> i32 {\n    a + b\n}\n";

fn security_project(source: &str) -> (Project, CheckArgs) {
    let project = Project::new();
    project.write("lib.rs", source);
    let mut options = args();
    options.rules = catalog::SECURITY.iter().map(|r| r.to_string()).collect();
    (project, options)
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
        (Status::Clear, 1)
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
    let project = Project::new();
    project.write(
        "server.ts",
        "const app = express()\napp.use(cors({ origin: true, credentials: true }))\n",
    );
    let mut options = args();
    options.rules = vec![catalog::UNSAFE_SETTINGS.into()];
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
fn a_foreign_error_message_confirms_an_error_detail_lean_as_a_consider() {
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
    assert_eq!(finding.strength, Strength::Consider);
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

const ROUTE: &str = "export async function loadThing(c: Context) {\n  const { data, error } = await db.from('things').select('*').eq('id', c.req.param('id'))\n  if (error) throw new InternalError(`Query failed: ${error.message}`, error)\n  if (!data) throw new NotFoundError('Thing not found')\n  return c.json(data)\n}\n";

fn choice_of(chosen: &str, options: &[&str]) -> Value {
    let probabilities: serde_json::Map<String, Value> = options
        .iter()
        .map(|o| {
            (
                o.to_string(),
                json!(if *o == chosen {
                    0.9
                } else {
                    0.1 / (options.len() - 1) as f64
                }),
            )
        })
        .collect();
    json!({"type":"choice","choice":chosen,"confidence":0.9,"probabilities":probabilities})
}

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
    assert_eq!(finding.strength, Strength::Consider);
    assert!(
        finding.message.contains("into an error message (0.90)")
            && finding
                .message
                .ends_with("The message is `Query failed: ${error.message}`."),
        "{}",
        finding.message
    );
}

#[test]
fn a_registered_error_handler_is_one_unit_judged_with_the_error_classes() {
    let project = Project::new();
    project.write(
        "src/app.ts",
        "import { errorHandler } from './middleware/error-handler'\nconst app = new Hono()\napp.onError(errorHandler)\nexport default app\n",
    );
    project.write(
        "src/middleware/error-handler.ts",
        "export const errorHandler = (err, c) => {\n  logger.error(err)\n  if (err instanceof AppError) {\n    return c.json({ error: { code: err.code, message: err.message } }, err.status)\n  }\n  return c.json({ error: { message: err.message, stack: err.stack } }, 500)\n}\n",
    );
    project.write(
        "src/lib/errors.ts",
        "export class AppError extends Error {\n  constructor(public status: number, public code: string, message: string) {\n    super(message)\n  }\n}\n",
    );
    project.write(
        "src/app.test.ts",
        "import { errorHandler } from './middleware/error-handler'\ntest('x', () => {\n  app.onError(errorHandler)\n})\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (inputs, plan) = planned(&project, &options);
    let handlers: Vec<(&std::path::Path, &UnitPlan)> = plan
        .files
        .values()
        .flat_map(|f| f.units.iter().map(move |u| (f.path.as_path(), u)))
        .filter(|(_, u)| matches!(u.detail, Detail::Handler { .. }))
        .collect();
    assert_eq!(handlers.len(), 1, "registered once outside tests");
    let (path, unit) = handlers[0];
    assert_eq!(
        path,
        std::path::Path::new("src/middleware/error-handler.ts")
    );
    assert_eq!(unit.name, "errorHandler");
    let request = &plan
        .requests
        .iter()
        .find(|p| p.request["state"]["error_handler"].is_object())
        .unwrap()
        .request;
    assert_eq!(
        request["state"]["error_handler"]["registered"],
        "`app.onError(errorHandler)` (src/app.ts:3)"
    );
    assert!(
        request["state"]["error_classes"]
            .as_str()
            .unwrap()
            .starts_with("export class AppError")
    );
    assert_eq!(request["jevgate"]["sources"].as_array().unwrap().len(), 2);
    assert!(inputs.len() >= 3);
    let report = run_with_nouls(&project, &options, &[("handler_leaks", 0.95)]);
    let file = report
        .files
        .iter()
        .find(|f| f.path.ends_with("error-handler.ts"))
        .unwrap();
    let finding = file
        .findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("errorHandler") && f.strength == Strength::Review)
        .unwrap();
    assert!(
        finding
            .message
            .starts_with("`errorHandler`, the error handler registered by `app.onError(errorHandler)` (src/app.ts:3), sends clients"),
        "{}",
        finding.message
    );
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-209 error details exposed")
    );
}

#[test]
fn registrations_named_in_comments_or_strings_register_nothing() {
    let project = Project::new();
    project.write(
        "src/patterns.ts",
        "// Handlers are found where the program calls `.onError(handler)`.\nexport const REGISTRATIONS = ['.onError(', '.setErrorHandler(']\nexport function describe(app) {\n  return `app.onError(report)` + app.name\n}\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    assert!(
        !plan
            .files
            .values()
            .flat_map(|f| &f.units)
            .any(|u| matches!(u.detail, Detail::Handler { .. }))
    );
}

#[test]
fn framework_error_handlers_are_found_where_they_are_implemented_or_used() {
    let project = Project::new();
    project.write(
        "src/error.rs",
        "use axum::response::{IntoResponse, Response};\n\n#[derive(thiserror::Error, Debug)]\npub enum Error {\n    #[error(\"request path not found\")]\n    NotFound,\n    #[error(\"an internal server error occurred\")]\n    Anyhow(#[from] anyhow::Error),\n}\n\n#[derive(Debug)]\npub struct TimeoutError;\n\nimpl IntoResponse for Error {\n    fn into_response(self) -> Response {\n        (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()).into_response()\n    }\n}\n\nimpl IntoResponse for Page {\n    fn into_response(self) -> Response {\n        Html(self.0).into_response()\n    }\n}\n",
    );
    project.write(
        "src/server.ts",
        "const app = express()\napp.use(express.json())\napp.use(cors({ origin: true }))\napp.use((err: Error, req: Request<{}, any>, res: Response, next: NextFunction) => {\n  res.status(500).json({ message: err.message })\n})\n",
    );
    project.write(
        "src/filter.ts",
        "@Catch(HttpException)\nexport class HttpErrorFilter implements ExceptionFilter {\n  catch(exception: HttpException, host: ArgumentsHost) {\n    host.switchToHttp().getResponse().status(500).json(exception.getResponse())\n  }\n}\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    let registered: Vec<String> = plan
        .files
        .values()
        .flat_map(|f| &f.units)
        .filter_map(|u| match &u.detail {
            Detail::Handler { registered } => Some(registered.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(registered.len(), 3, "{registered:?}");
    assert!(
        registered.contains(&"`impl IntoResponse for Error` (src/error.rs:15)".to_string()),
        "{registered:?}"
    );
    assert!(
        registered
            .iter()
            .any(|r| r.starts_with("`app.use((err: Error"))
    );
    assert!(
        registered.contains(&"`@Catch(…) class HttpErrorFilter` (src/filter.ts:3)".to_string())
    );
    let classes = plan
        .requests
        .iter()
        .find(|p| {
            p.request["state"]["error_handler"]["registered"]
                .as_str()
                .is_some_and(|r| r.contains("IntoResponse"))
        })
        .unwrap()
        .request["state"]["error_classes"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        classes.starts_with("#[derive(thiserror::Error, Debug)]\npub enum Error {"),
        "{classes}"
    );
    assert!(classes.contains("an internal server error occurred"));
    assert!(classes.ends_with("pub struct TimeoutError;"), "{classes}");
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
