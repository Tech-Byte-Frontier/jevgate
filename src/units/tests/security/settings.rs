//! Unsafe settings: a file's top-level setup as one unit, C# setup traces, the
//! token, key, escaping and password checks, and broad answers no check names.
use super::*;

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
pub(super) struct Recording {
    pub(super) inner: Scripted,
    pub(super) requests: Vec<Value>,
}

impl crate::transport::Evaluator for Recording {
    fn evaluate(&mut self, request: &Value) -> anyhow::Result<Value> {
        self.requests.push(request.clone());
        self.inner.evaluate(request)
    }
}

pub(super) fn recording(nouls: &[(&'static str, f64)]) -> Recording {
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
pub(super) fn settings_run(
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
pub(super) fn trace_questions(requests: &[Value]) -> serde_json::Map<String, Value> {
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
