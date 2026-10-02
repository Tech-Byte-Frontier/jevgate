//! Sensitive data: error details settled by where their text goes, the error
//! messages a function creates or its callees raise, development-only exposure
//! and audit lines.
use super::*;

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
fn development_only_exposure_is_one_level_lower_and_names_its_weakness() {
    let (project, options) = security_project(QUERY);
    let report = run_with_nouls(
        &project,
        &options,
        &[("logs_secret", 0.95), ("dev_only", 0.95)],
    );
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.rule, "security/sensitive-data");
    assert_eq!(composed(finding), Strength::Consider);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-532 sensitive data in logs")
    );
    assert!(finding.message.contains("runs only in development"));
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
    // Leaning toward a client, it is a note, which one level does not report.
    assert!(report.files[0].findings.is_empty());
    assert_eq!(status(&report), Status::Clear);
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
    // Where the text goes is still undecided: a note, not reported.
    assert!(report.files[0].findings.is_empty());
    assert_eq!(
        report.files[0].dimensions[catalog::SENSITIVE_DATA].status,
        Status::Clear
    );
}

pub(super) const SERVICE: &str = "def find_asset(asset_id):\n    asset = ASSETS.get(asset_id)\n    if asset is None:\n        raise LookupError(\"Asset not found\")\n    return asset\n";
pub(super) const HANDLER: &str = "from fastapi import HTTPException\n\nfrom app.services import find_asset\n\n\ndef read_asset(asset_id: str):\n    try:\n        return find_asset(asset_id)\n    except LookupError as exc:\n        raise HTTPException(status_code=404, detail=str(exc)) from exc\n";

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
        trace: Some(trace), ..
    } = &plan.files[&owner].units[0].detail
    else {
        panic!("a traced security unit");
    };
    let trace = trace.request();
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
        names_callees(&trace),
        "passing on a callee's own error text is the program's own"
    );
    let alone = project_with(&[("app/api.py", HANDLER)], &[catalog::SENSITIVE_DATA]);
    let (_, plan) = planned(&alone.0, &alone.1);
    let Detail::Security {
        trace: Some(trace), ..
    } = &plan.files[&0].units[0].detail
    else {
        panic!("a traced security unit");
    };
    let trace = trace.request();
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
    assert!(trace["questions"].get("messages").is_some() && !names_callees(&trace));
}

pub(super) const ROUTE: &str = "export async function loadThing(c: Context) {\n  const { data, error } = await db.from('things').select('*').eq('id', c.req.param('id'))\n  if (error) throw new InternalError(`Query failed: ${error.message}`, error)\n  if (!data) throw new NotFoundError('Thing not found')\n  return c.json(data)\n}\n";

#[test]
fn each_created_error_message_is_asked_about_and_names_the_foreign_one() {
    let project = Project::new();
    project.write("routes.ts", ROUTE);
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    let Detail::Security {
        trace: Some(trace),
        messages,
        ..
    } = &plan.files[&0].units[0].detail
    else {
        panic!("a traced security unit");
    };
    let trace = trace.request();
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
    // Where the text goes is still undecided: a note, not reported.
    assert!(report.files[0].findings.is_empty());
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
