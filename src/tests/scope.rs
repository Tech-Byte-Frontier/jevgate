//! What a run judges: unsupported and oversized input is skipped, an empty
//! scope is incomplete, and only selected roles and context are uploaded.
use super::*;

#[test]
fn unparseable_binary_and_unsupported_files_are_skipped_without_blocking_the_run() {
    let project = Project::new();
    project.write("large.rs", &function("too_large"));
    project.write("invalid.rs", "fn broken( {");
    project.write("main.zig", "pub fn main() void {}\n");
    project.write("ok.rs", &function("ok"));
    std::fs::write(project.0.join("latin1.rs"), b"fn caf\xe9() {}\n").unwrap();
    let mut options = args();
    options.max_file_bytes = 160;
    assert!(function("ok").len() <= 160 && function("too_large").len() > 160);
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    assert_eq!(mock.calls, 1);
    assert!(report.complete, "{:?}", report.files);
    let file = |name: &str| {
        report
            .files
            .iter()
            .find(|f| f.path.ends_with(name))
            .unwrap()
    };
    assert_eq!(file("ok.rs").status, schema::Status::Clear);
    for name in ["invalid.rs", "main.zig", "latin1.rs"] {
        assert_eq!(file(name).status, schema::Status::Skipped, "{name}");
        assert!(
            file(name).error.as_ref().unwrap().contains("not judged"),
            "{name}"
        );
    }
    let large = file("large.rs");
    assert_eq!(large.status, schema::Status::NeedsContext);
    assert!(large.dimensions.is_empty() && large.findings.is_empty());
    let reason = &large.classification.as_ref().unwrap().reason;
    assert!(
        reason.contains("too_large") && reason.contains("160-byte read cap"),
        "{reason}"
    );
    assert_eq!(report.status, "needs-context");
    assert_eq!(gate::exit_code(&report), 0);
}

#[test]
fn a_partly_broken_file_is_judged_for_its_intact_units_and_names_the_rest() {
    let project = Project::new();
    // tree-sitter-rust reads snapbox's `str![…]` as the type `str`.
    let misread = function("broken").replace(
        "let doubled = total * 2;",
        "let doubled = str![[\"x\"]].len() as i32 * total;",
    );
    project.write("lib.rs", &format!("{}{misread}", function("kept")));
    let mut mock = Mock::default();
    let mut report = run(&project, &args(), &mut mock);
    assert!(report.complete);
    let file = &report.files[0];
    assert_eq!(file.status, schema::Status::Clear);
    assert_eq!(
        file.left_out,
        [schema::LeftOut {
            unit: "broken".into(),
            start_line: 9,
            end_line: 16,
            reason: "Syntax error at line 14.".into(),
        }]
    );
    assert!(
        mock.requests
            .iter()
            .all(|r| !r.to_string().contains("fn broken"))
    );
    let text = |report: &schema::Report, verbose: bool| {
        let mut out = Vec::new();
        crate::output::agent(&mut out, report, verbose, crate::output::Style::PLAIN).unwrap();
        String::from_utf8(out).unwrap()
    };
    assert!(
        text(&report, false).contains(
            "\nLeft out over syntax errors, the rest of each file judged: 1 unit in 1 file.\n  lib.rs:9 broken: Syntax error at line 14.\n"
        ),
        "{}",
        text(&report, false)
    );
    // Ten are listed; `--verbose` lists them all.
    let entry = report.files[0].left_out[0].clone();
    report.files[0].left_out = (1..=12)
        .map(|line| schema::LeftOut {
            unit: String::new(),
            start_line: line,
            end_line: line,
            ..entry.clone()
        })
        .collect();
    let short = text(&report, false);
    assert!(short.contains("  lib.rs:10 line 10: ") && !short.contains("lib.rs:11 "));
    assert!(
        short.contains("  … 2 more; --verbose lists all.\n"),
        "{short}"
    );
    assert!(text(&report, true).contains("  lib.rs:12 line 12: "));
}

#[test]
fn a_function_too_large_for_one_request_is_needs_context_and_not_sent() {
    let project = Project::new();
    let mut body = String::from("fn huge() -> usize {\n    let mut total = 0;\n");
    let mut index = 0usize;
    while body.len() < 200_000 {
        body.push_str(&format!("    total += {index} * {index};\n"));
        index += 1;
    }
    body.push_str("    total\n}\n");
    project.write("huge.rs", &format!("{body}\n{}", function("small")));
    let mut options = args();
    options.max_file_bytes = 1_048_576;
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    assert!(report.complete);
    let huge = &report.files[0];
    let dimension = &huge.dimensions["function_simplification"];
    assert_eq!(dimension.units.needs_context, 1);
    assert_eq!(dimension.units.judged, 1);
    assert_eq!(dimension.status, schema::Status::NeedsContext);
    assert!(
        mock.requests
            .iter()
            .all(|r| !r["state"]["functions"].to_string().contains("fn huge"))
    );
    assert_eq!(huge.status, schema::Status::NeedsContext);
}

#[test]
fn empty_scope_is_incomplete() {
    let project = Project::new();
    let report = run(&project, &args(), &mut Mock::default());
    assert!(!report.complete);
    assert!(!report.acceptance_evaluated);
}

#[test]
fn fixture_and_generated_roles_are_not_uploaded() {
    let project = Project::new();
    std::fs::create_dir(project.0.join("fixtures")).unwrap();
    project.write("fixtures/sample.rs", "fn fixture() {}");
    project.write("database.types.ts", "export type Db = string;");
    let mut context = project.context();
    context.config.generated = vec!["database.types.ts".into()];
    let inputs = inventory::collect(&args(), &context, &[]).unwrap();
    for path in ["fixtures/sample.rs", "database.types.ts"] {
        let input = inputs
            .iter()
            .find(|i| i.result.path == std::path::Path::new(path))
            .unwrap();
        assert_eq!(input.result.status, schema::Status::Skipped, "{path}");
        assert!(input.source.is_none(), "{path}");
    }
}

#[cfg(unix)]
#[test]
fn context_limits_and_visibility_are_enforced_without_api_calls() {
    let project = Project::new();
    project.write("lib.rs", "fn f() {}");
    project.write("contract.md", "a contract");
    project.write(".env", "TYPESAFE_API_KEY=secret");
    let mut options = args();
    options.context.push(".env".into());
    assert!(inventory::collect(&options, &project.context(), &[]).is_err());
    options.context = vec!["contract.md".into()];
    options.max_context_bytes = 2;
    assert!(inventory::collect(&options, &project.context(), &[]).is_err());
    options.max_context_bytes = 100;
    std::fs::remove_file(project.0.join("contract.md")).unwrap();
    assert!(inventory::collect(&options, &project.context(), &[]).is_err());
}

/// A Kotlin function with five body lines and a comment, in `src/shop.kt`.
const KOTLIN_SHOP: &str = "package shop\n\n// Totals the open orders, the larger first.\nfun openTotal(orders: List<Order>): Int {\n    val open = orders.filter { it.open }\n    val sorted = open.sortedByDescending { it.total }\n    var total = 0\n    for (order in sorted) {\n        total += order.total\n    }\n    return total\n}\n";

#[test]
fn a_generic_language_gets_only_the_rules_its_units_serve_and_its_tests_are_not_judged() {
    let project = Project::new();
    project.write("lib.rs", &function("a"));
    project.write("src/shop.kt", KOTLIN_SHOP);
    project.write(
        "src/test/kotlin/ShopTest.kt",
        "class ShopTest {\n    fun totals() {\n        check(openTotal(listOf()) == 0)\n    }\n}\n",
    );
    let mut options = args();
    options.include_tests = true;
    let mut mock = Mock::default();
    let report = run(&project, &options, &mut mock);
    assert!(report.complete, "{:?}", report.files);
    let file = |name: &str| {
        report
            .files
            .iter()
            .find(|f| f.path.ends_with(name))
            .unwrap()
    };
    let shop = file("shop.kt");
    let rules: Vec<&str> = shop.dimensions.keys().map(String::as_str).collect();
    assert_eq!(
        rules,
        [
            "comments",
            "file_organization",
            "function_simplification",
            "shared_logic"
        ]
    );
    let reason = &shop.classification.as_ref().unwrap().reason;
    assert!(
        reason.contains("the hardcoded-value, security and test rules do not read Kotlin"),
        "{reason}"
    );
    let test = file("ShopTest.kt");
    assert_eq!(test.status, schema::Status::NotApplicable);
    assert_eq!(
        test.classification.as_ref().unwrap().reason,
        "Test file. JevGate does not judge Kotlin tests yet."
    );
    let kotlin: Vec<&serde_json::Value> = mock
        .requests
        .iter()
        .filter(|r| r["state"]["file"]["path"] == "src/shop.kt")
        .collect();
    assert!(!kotlin.is_empty());
    assert!(
        kotlin
            .iter()
            .all(|r| r["state"]["file"]["language"] == "Kotlin")
    );
    assert!(
        mock.requests
            .iter()
            .all(|r| !r.to_string().contains("ShopTest")),
        "a test file of the generic tier sends nothing, not even its purpose"
    );
    // The Rust file is asked exactly what it is asked alone.
    let alone = Project::new();
    alone.write("lib.rs", &function("a"));
    let mut solo = Mock::default();
    run(&alone, &options, &mut solo);
    let rust = |requests: &[serde_json::Value]| -> Vec<String> {
        requests
            .iter()
            .filter(|r| r["state"]["file"]["path"] == "lib.rs")
            .map(|r| r.to_string())
            .collect()
    };
    assert!(!rust(&solo.requests).is_empty());
    assert_eq!(rust(&mock.requests), rust(&solo.requests));
}
