//! Next.js: file roles sent beside the path, open redirects, secrets in
//! public environment variables and `next.config` settings.
use super::*;

const MANIFEST: &str =
    "{\"name\": \"web\", \"dependencies\": {\"next\": \"15.3.0\", \"react\": \"19.1.0\"}}\n";

const ROUTE: &str = "import { NextRequest, NextResponse } from 'next/server';\n\nexport async function GET(request: NextRequest) {\n  const target = request.nextUrl.searchParams.get('url');\n  const upstream = await fetch(target!);\n  return new NextResponse(await upstream.text());\n}\n";

fn next_project(files: &[(&str, &str)]) -> (Project, CheckArgs) {
    let project = Project::new();
    project.write("package.json", MANIFEST);
    for (path, source) in files {
        project.write(path, source);
    }
    let mut options = args();
    options.rules = catalog::SECURITY.iter().map(|r| r.to_string()).collect();
    (project, options)
}

/// The requests planned for the file at `path`.
fn requests_of<'a>(plan: &'a Plan, path: &str) -> Vec<&'a Value> {
    plan.requests
        .iter()
        .map(|p| &p.request)
        .filter(|r| r["state"]["file"]["path"] == path)
        .collect()
}

#[test]
fn next_files_carry_their_role_and_every_question_points_to_it() {
    let helper = "export async function load(url: string) {\n  const response = await fetch(url);\n  return response.json();\n}\n";
    let (project, options) =
        next_project(&[("app/api/proxy/route.ts", ROUTE), ("lib/load.ts", helper)]);
    let (_, plan) = planned(&project, &options);
    let route = requests_of(&plan, "app/api/proxy/route.ts");
    assert!(!route.is_empty());
    for request in route {
        let framework = request["state"]["file"]["framework"].as_str().unwrap();
        assert!(
            framework.starts_with("Next.js App Router route handler"),
            "{framework}"
        );
        for question in request["questions"].as_object().unwrap().values() {
            let note = question["instructions"]["note"].as_str().unwrap();
            assert!(note.starts_with("`file.framework`"), "{note}");
        }
    }
    let library = requests_of(&plan, "lib/load.ts");
    assert!(!library.is_empty());
    for request in library {
        assert!(request["state"]["file"]["framework"].is_null());
        assert!(!request.to_string().contains("file.framework"));
    }
}

#[test]
fn an_unchecked_redirect_target_is_an_open_redirect_review() {
    let middleware = "import { NextResponse, type NextRequest } from 'next/server';\n\nexport function middleware(request: NextRequest) {\n  const returnTo = request.nextUrl.searchParams.get('returnTo');\n  if (returnTo) {\n    return NextResponse.redirect(returnTo);\n  }\n  return NextResponse.next();\n}\n";
    let (project, mut options) = next_project(&[("middleware.ts", middleware)]);
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("redirect", noul_at(0.95)),
        ("origin", spread(0.0, 0.05, 0.95)),
    ];
    let report = run(&project, &options, &mut eval);
    let file = report
        .files
        .iter()
        .find(|f| f.path == std::path::Path::new("middleware.ts"))
        .unwrap();
    let finding = &file.findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(finding.category.as_deref(), Some("CWE-601 open redirect"));
    assert!(finding.message.contains("a URL it redirects clients to"));
    assert!(finding.action.contains("paths on this site"));
}

#[test]
fn a_parameter_only_redirect_is_a_note_until_callers_show_another_party() {
    let helper = "export function goTo(path: string) {\n  const target = `/app/${path}`;\n  redirect(target);\n}\n";
    let (project, mut options) = next_project(&[("lib/navigation.ts", helper)]);
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("redirect", noul_at(0.95)),
        ("origin", spread(0.0, 0.95, 0.05)),
    ];
    let report = run(&project, &options, &mut eval);
    let file = report
        .files
        .iter()
        .find(|f| f.path == std::path::Path::new("lib/navigation.ts"))
        .unwrap();
    assert_eq!(file.findings[0].strength, Strength::Note);
}

#[test]
fn a_secret_in_a_public_environment_variable_is_an_unsafe_setting() {
    let stripe = "import Stripe from 'stripe';\n\nexport const stripe = new Stripe(process.env.NEXT_PUBLIC_STRIPE_SECRET_KEY!, {\n  apiVersion: '2025-04-30.basil',\n});\n";
    let (project, mut options) = next_project(&[("lib/stripe.ts", stripe)]);
    options.rules = vec![catalog::UNSAFE_SETTINGS.into()];
    let report = run_with_nouls(
        &project,
        &options,
        &[("weakened", 0.95), ("public_secret", 0.95)],
    );
    let file = report
        .files
        .iter()
        .find(|f| f.path == std::path::Path::new("lib/stripe.ts"))
        .unwrap();
    let finding = &file.findings[0];
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-200 secret exposed to browsers")
    );
    assert!(
        finding
            .message
            .starts_with("Module setup reads a secret from an environment variable"),
        "{}",
        finding.message
    );
}

#[test]
fn next_config_objects_are_one_module_unit_for_unsafe_settings() {
    let config = "import type { NextConfig } from 'next';\n\nconst nextConfig: NextConfig = {\n  reactStrictMode: true,\n  async headers() {\n    return [\n      {\n        source: '/api/:path*',\n        headers: [\n          { key: 'Access-Control-Allow-Origin', value: '*' },\n          { key: 'Access-Control-Allow-Credentials', value: 'true' },\n        ],\n      },\n    ];\n  },\n};\n\nexport default nextConfig;\n";
    let (project, mut options) = next_project(&[("next.config.ts", config)]);
    options.rules = vec![catalog::UNSAFE_SETTINGS.into()];
    let (_, plan) = planned(&project, &options);
    let requests = requests_of(&plan, "next.config.ts");
    assert_eq!(requests.len(), 1, "one module request, no function units");
    let module = requests[0]["state"]["module"]["source"].as_str().unwrap();
    assert!(module.contains("'Access-Control-Allow-Origin', value: '*'"));
    assert!(!module.contains("export default"));
    assert!(
        requests[0]["state"]["file"]["framework"]
            .as_str()
            .unwrap()
            .starts_with("Next.js configuration")
    );
    let report = run_with_nouls(&project, &options, &[("weakened", 0.95), ("cors", 0.95)]);
    let file = report
        .files
        .iter()
        .find(|f| f.path == std::path::Path::new("next.config.ts"))
        .unwrap();
    assert_eq!(
        file.findings[0].category.as_deref(),
        Some("CWE-942 permissive CORS")
    );
}

#[test]
fn only_questions_about_values_carry_the_framework_role() {
    let route = "export async function GET(request: Request) {\n  const url = new URL(request.url);\n  const name = url.searchParams.get('name') ?? 'guest';\n  if (name.length > 40) {\n    return new Response('too long', { status: 400 });\n  }\n  const greeting = `Hello, ${name}`;\n  return new Response(greeting);\n}\n";
    let (project, mut options) = next_project(&[("app/hello/route.ts", route)]);
    options.rules.push(catalog::FUNCTION_SIMPLIFICATION.into());
    let (_, plan) = planned(&project, &options);
    let requests = requests_of(&plan, "app/hello/route.ts");
    let stage = |r: &Value| r["jevgate"]["stage"].as_str().unwrap().to_string();
    let with_role: Vec<String> = requests
        .iter()
        .filter(|r| r["state"]["file"]["framework"].is_string())
        .map(|r| stage(r))
        .collect();
    let without: Vec<String> = requests
        .iter()
        .filter(|r| r["state"]["file"]["framework"].is_null())
        .map(|r| stage(r))
        .collect();
    assert_eq!(with_role, ["security"]);
    assert_eq!(without, ["functions"], "how code reads needs no role");
}

const PORTAL: &str = "'use server';\n\nimport { redirect } from 'next/navigation';\n\nexport async function goToSection(section: string) {\n  redirect(`/account/${section}?tab=billing`);\n}\n";

const TARGETS: [&str; 5] = ["own", "checked", "given", "outside", "none"];

/// The injection status of `app/actions.ts` and its settle requests, with
/// the redirect check undecided and the target Choice answering `chosen`.
fn settled_redirect(project: &Project, options: &CheckArgs, chosen: &str) -> (Status, u64) {
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("resource", noul_at(0.95)),
        ("redirect", noul_at(0.4)),
        ("origin", spread(0.0, 0.9, 0.1)),
        ("redirect_target", choice_of(chosen, &TARGETS)),
    ];
    let report = run(project, options, &mut eval);
    let file = report
        .files
        .iter()
        .find(|f| f.path == std::path::Path::new("app/actions.ts"))
        .unwrap();
    (
        file.dimensions[catalog::INJECTION].status.clone(),
        report
            .stages
            .get("settle")
            .map_or(0, |stage| stage.successful_requests),
    )
}

#[test]
fn an_undecided_redirect_is_settled_by_where_its_target_comes_from() {
    let (project, mut options) = next_project(&[("app/actions.ts", PORTAL)]);
    options.rules = vec![catalog::INJECTION.into()];
    assert_eq!(
        settled_redirect(&project, &options, "own"),
        (Status::Clear, 1)
    );
    for chosen in ["given", "outside"] {
        options.refresh = true;
        assert_eq!(
            settled_redirect(&project, &options, chosen),
            (Status::Uncertain, 1),
            "{chosen}"
        );
    }
}

#[test]
fn a_url_note_clears_when_the_request_leaves_from_the_browser() {
    let search = "'use client';\n\nexport function Search({ endpoint }: { endpoint: string }) {\n  async function run(term: string) {\n    const response = await fetch(`${endpoint}?q=${encodeURIComponent(term)}`);\n    return response.json();\n  }\n  return <input onChange={(event) => run(event.target.value)} />;\n}\n";
    let (project, mut options) = next_project(&[("components/search.tsx", search)]);
    options.rules = vec![catalog::INJECTION.into()];
    let findings = |runs_in: &str, options: &CheckArgs| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("resource", noul_at(0.95)),
            ("url", noul_at(0.6)),
            ("origin", spread(0.0, 0.9, 0.1)),
            (
                "runs_in",
                choice_of(runs_in, &["browser", "server", "either"]),
            ),
        ];
        let report = run(&project, options, &mut eval);
        let file = report
            .files
            .iter()
            .find(|f| f.path == std::path::Path::new("components/search.tsx"))
            .unwrap();
        file.findings.iter().map(|f| f.strength).collect::<Vec<_>>()
    };
    assert!(findings("browser", &options).is_empty());
    options.refresh = true;
    assert_eq!(findings("server", &options), [Strength::Note]);
}
