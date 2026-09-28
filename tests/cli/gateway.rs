//! Gateway keys end to end: checks with an OpenRouter or Vercel AI Gateway
//! key against a mock provider in each gateway's shape, and `auth` with them.
use super::*;
use mock_provider::{MockProvider, Received, Reply, answer};
use serde_json::{Value, json};

/// OpenRouter's System One API as its docs show it: the OpenRouter model id,
/// its own request `id`, the `provider` and a `usage` with `cost`; and a
/// model list, as a proxy of TypeSafe's API serves.
fn openrouter() -> MockProvider {
    MockProvider::start(|received| {
        if received.path.ends_with("/v1/models") {
            return Reply::json(200, &json!({"models": [{"name": "typesafe/jev-1.13"}]}));
        }
        let mut body = answer(&received.json(), 0);
        body["model"] = json!("typesafe/jev-1.13");
        body["id"] = json!("gen-dec-1789738314-X5e5");
        body["provider"] = json!("TypeSafe");
        body["usage"]["cost"] = json!(0.00000042);
        Reply::json(200, &body)
    })
}

/// A gateway that answers without `usage`, naming its own alias, with
/// TypeSafe's request id header.
fn vercel() -> MockProvider {
    MockProvider::start(|received| {
        let mut body = answer(&received.json(), 0);
        body["model"] = json!("typesafe-ai/jev");
        body.as_object_mut().unwrap().remove("usage");
        Reply::json(200, &body).header("x-typesafe-request-id", "req_vercel_1")
    })
}

/// A project with one judged function, checked with `variable` set to `key`
/// and requests sent to `root`.
fn check(
    project: &Project,
    (variable, key): (&str, &str),
    root: &str,
    format: &str,
) -> std::process::Output {
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    project
        .command()
        .args(["check", ".", "--format", format])
        .env(variable, key)
        .env("JEVGATE_BASE_URL", root)
        .output()
        .unwrap()
}

/// Every request is a question to `path` with the bearer `key` and `model`.
fn asked(received: &[Received], path: &str, key: &str, model: &str) {
    assert!(!received.is_empty());
    for request in received {
        assert_eq!(
            (request.method.as_str(), request.path.as_str()),
            ("POST", path)
        );
        assert_eq!(
            request.header("authorization"),
            Some(format!("Bearer {key}").as_str())
        );
        assert_eq!(request.json()["model"], model);
        assert!(
            !request.body.contains("\"jevgate\""),
            "local metadata stays local"
        );
    }
}

#[test]
fn a_check_with_an_openrouter_key_asks_openrouter_for_its_model() {
    let project = Project::new();
    let provider = openrouter();
    let root = format!("{}/api", provider.url);
    let output = check(
        &project,
        ("OPENROUTER_API_KEY", "sk-or-v1-test"),
        &root,
        "json",
    );
    let errors = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{errors}");
    assert!(errors.contains(&format!("Sending requests to {root} (JEVGATE_BASE_URL)")));
    let received = provider.received();
    asked(
        &received,
        "/api/v1/systemone",
        "sk-or-v1-test",
        "typesafe/jev-1.13",
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["provider"], "openrouter");
    assert_eq!(report["concurrency"], 3, "a gateway's default");
    assert_eq!(report["requested_model"], "typesafe/jev-1.13");
    assert_eq!(
        report["paid_models"]["typesafe/jev-1.13"],
        10 * received.len()
    );
    assert!(report["estimated_usd"].as_f64().unwrap() > 0.0);
    let judgment = &report["files"][0]["judgments"][0];
    assert_eq!(judgment["request_id"], "gen-dec-1789738314-X5e5");
}

#[test]
fn a_check_with_a_vercel_key_shows_the_gateway_and_an_unknown_cost() {
    let project = Project::new();
    let provider = vercel();
    let root = format!("{}/typesafe", provider.url);
    let output = check(&project, ("AI_GATEWAY_API_KEY", "vck_test"), &root, "agent");
    let text = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{text}");
    let headline = text.lines().next().unwrap();
    assert!(
        headline.contains("API requests via Vercel AI Gateway"),
        "{headline}"
    );
    assert!(
        headline.ends_with("· 0 input tokens · cost unknown"),
        "{headline}"
    );
    asked(
        &provider.received(),
        "/typesafe/v1/systemone",
        "vck_test",
        "typesafe-ai/jev",
    );
    let report = project.snapshot().unwrap();
    assert_eq!(report["unmetered_requests"], provider.received().len());
    assert!(report["estimated_usd"].is_null());
    assert_eq!(
        report["files"][0]["judgments"][0]["request_id"],
        "req_vercel_1"
    );
}

#[cfg(unix)]
#[test]
fn a_gateway_key_exported_for_other_tools_does_not_replace_the_key_given_to_jevgate() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    std::fs::write(project.0.join("ts.env"), "TYPESAFE_API_KEY=file-key\n").unwrap();
    project.save_credential("saved-key");
    for (args, key) in [
        (&["--env-file", "ts.env"][..], "file-key"),
        (&[], "saved-key"),
    ] {
        let provider =
            MockProvider::start(|received| Reply::json(200, &answer(&received.json(), 0)));
        let output = project
            .command()
            .args(["check", ".", "--refresh", "--format", "json"])
            .args(args)
            .env("OPENROUTER_API_KEY", "sk-or-v1-other-tool")
            .env("JEVGATE_BASE_URL", &provider.url)
            .output()
            .unwrap();
        let errors = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(0), "{errors}");
        asked(&provider.received(), "/v1/systemone", key, "jev-1.13.0");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["provider"], "typesafe", "{key}");
    }
    let status = project
        .command()
        .args(["auth", "status", "--offline", "--json"])
        .env("OPENROUTER_API_KEY", "sk-or-v1-other-tool")
        .output()
        .unwrap();
    let body: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert!(
        body["source"]
            .as_str()
            .unwrap()
            .starts_with("protected file:")
    );
    assert_eq!(
        body["unused"],
        json!(["OPENROUTER_API_KEY environment variable"])
    );
}

#[test]
fn the_repository_cannot_choose_where_keys_go() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    for (config, variable) in [
        (
            "base_url = \"https://attacker.example\"\n",
            "TYPESAFE_API_KEY",
        ),
        ("provider = \"openrouter\"\n", "TYPESAFE_API_KEY"),
    ] {
        std::fs::write(project.0.join("jevgate.toml"), config).unwrap();
        let output = project
            .command()
            .args(["check", "."])
            .env(variable, "private-key")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{config}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unknown field"),
            "{config}"
        );
    }
    std::fs::remove_file(project.0.join("jevgate.toml")).unwrap();
    for root in ["http://attacker.example", "https://user@attacker.example"] {
        let output = project
            .command()
            .args(["check", "."])
            .env("TYPESAFE_API_KEY", "private-key")
            .env("JEVGATE_BASE_URL", root)
            .output()
            .unwrap();
        let errors = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{root}");
        assert!(
            errors.contains("JEVGATE_BASE_URL must be an https:// URL"),
            "{errors}"
        );
        assert!(!errors.contains("private-key"));
    }
}

#[test]
fn auth_status_names_the_provider_and_the_keys_set_but_not_used() {
    let project = Project::new();
    let output = project
        .command()
        .args(["auth", "status", "--offline", "--json"])
        .env("OPENROUTER_API_KEY", "sk-or-v1-private")
        .env("AI_GATEWAY_API_KEY", "vck_private")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private"));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["source"], "OPENROUTER_API_KEY environment variable");
    assert_eq!(body["provider"], "openrouter");
    assert_eq!(body["endpoint"], "https://openrouter.ai/api");
    assert_eq!(
        body["unused"],
        json!(["AI_GATEWAY_API_KEY environment variable"])
    );
    let provider = openrouter();
    let checked = project
        .command()
        .args(["auth", "status"])
        .env("OPENROUTER_API_KEY", "sk-or-v1-private")
        .env("JEVGATE_BASE_URL", format!("{}/api", provider.url))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&checked.stdout);
    assert!(checked.status.success(), "{text}");
    assert!(
        text.contains("Connection: authenticated with OpenRouter."),
        "{text}"
    );
    assert_eq!(provider.received()[0].path, "/api/v1/models");
}

#[test]
fn a_gateway_key_sends_three_requests_at_once_unless_told_otherwise() {
    let project = Project::new();
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    // The `concurrency` a dry run reports with `key` in its variable.
    let concurrency = |(variable, key): (&str, &str), args: &[&str]| {
        let output = project
            .command()
            .args(["check", "--dry-run", "--format", "json"])
            .args(args)
            .env(variable, key)
            .output()
            .unwrap();
        let errors = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{errors}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["concurrency"].clone()
    };
    let (typesafe, openrouter, vercel) = (
        ("TYPESAFE_API_KEY", "tsk-test"),
        ("OPENROUTER_API_KEY", "sk-or-v1-test"),
        ("AI_GATEWAY_API_KEY", "vck_test"),
    );
    assert_eq!(concurrency(typesafe, &[]), 6);
    assert_eq!(concurrency(openrouter, &[]), 3);
    assert_eq!(concurrency(vercel, &[]), 3);
    assert_eq!(concurrency(openrouter, &["--concurrency", "6"]), 6);
    std::fs::write(project.0.join("jevgate.toml"), "concurrency = 4\n").unwrap();
    assert_eq!(concurrency(vercel, &[]), 4);
    assert_eq!(concurrency(typesafe, &[]), 4);
}

#[test]
fn a_gateway_key_in_the_repository_env_is_named_but_not_used() {
    let project = Project::new();
    std::fs::write(project.0.join(".env"), "OPENROUTER_API_KEY=sk-or-v1-app\n").unwrap();
    let body =
        project.unconfigured_status(&[], "OPENROUTER_API_KEY is read only with --env-file .env");
    assert!(!body.to_string().contains("sk-or-v1-app"));
    let output = project
        .command()
        .args([
            "auth",
            "status",
            "--offline",
            "--json",
            "--env-file",
            ".env",
        ])
        .output()
        .unwrap();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["provider"], "openrouter");
}

/// `jevgate auth login --with-key` with `args`, the key piped on stdin and
/// every request sent to `root`.
#[cfg(unix)]
fn login(project: &Project, args: &[&str], key: &str, root: &str) -> std::process::Output {
    use std::io::Write;
    let mut child = project
        .command()
        .args(["auth", "login", "--with-key"])
        .args(args)
        .env("JEVGATE_BASE_URL", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(key.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[cfg(unix)]
#[test]
fn login_saves_a_gateway_key_with_its_provider_and_checks_plan_for_it() {
    let project = Project::new();
    let provider = openrouter();
    let root = format!("{}/api", provider.url);
    let output = login(
        &project,
        &["--provider", "openrouter"],
        "sk-or-v1-login\n",
        &root,
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.starts_with("OpenRouter API key verified and saved in protected file:"));
    let check = &provider.received()[0];
    assert_eq!(check.header("authorization"), Some("Bearer sk-or-v1-login"));
    let saved = project.0.join("isolated-auth");
    assert_eq!(
        std::fs::read_to_string(saved.join("credentials")).unwrap(),
        "openrouter sk-or-v1-login"
    );
    assert_eq!(
        std::fs::read_to_string(saved.join("provider")).unwrap(),
        "openrouter"
    );
    std::fs::write(project.0.join("lib.rs"), JUDGED_RS).unwrap();
    let preview = project.preview(&["check", "--dry-run", "--format", "json"]);
    assert_eq!(preview["requested_model"], "typesafe/jev-1.13");
    assert_eq!(preview["provider"], "openrouter");
    let refused = login(&project, &[], "sk-or-v1-login\n", &root);
    let errors = String::from_utf8_lossy(&refused.stderr);
    assert_eq!(refused.status.code(), Some(2));
    assert!(
        errors.contains("--provider openrouter") && !errors.contains("v1-login"),
        "{errors}"
    );
}

#[test]
fn rules_test_and_rules_propose_ask_through_the_gateway_of_the_key() {
    let project = Project::new();
    std::fs::create_dir_all(project.0.join(".jevgate/questions")).unwrap();
    std::fs::write(
        project.0.join(".jevgate/questions/body-logs.toml"),
        "question = \"Does this function write a request body to a log?\"\nunit = \"function\"\n\n[[passing]]\npath = \"src/orders.rs\"\ncode = \"fn charge(req: &Request) {\\n    log(req.id());\\n}\\n\"\n",
    )
    .unwrap();
    std::fs::write(
        project.0.join("AGENTS.md"),
        "# Conventions\n\n- Never log request bodies.\n",
    )
    .unwrap();
    let provider = openrouter();
    let root = format!("{}/api", provider.url);
    let run = |args: &[&str]| {
        project
            .command()
            .args(args)
            .env("OPENROUTER_API_KEY", "sk-or-v1-test")
            .env("JEVGATE_BASE_URL", &root)
            .output()
            .unwrap()
    };
    let output = run(&["rules", "test", "--format", "json"]);
    let errors = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{errors}");
    let tested: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        (&tested["provider"], &tested["requested_model"]),
        (&json!("openrouter"), &json!("typesafe/jev-1.13"))
    );
    assert!(tested["estimated_usd"].as_f64().unwrap() > 0.0);
    let output = run(&["rules", "propose", "--format", "json"]);
    assert_eq!(output.status.code(), Some(0));
    let proposed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(proposed["estimated_usd"].as_f64().unwrap() > 0.0);
    asked(
        &provider.received(),
        "/api/v1/systemone",
        "sk-or-v1-test",
        "typesafe/jev-1.13",
    );
}
