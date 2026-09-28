//! Custom questions' examples: planned as a check plans the same file.
use super::{
    custom::{configured, git},
    *,
};
use crate::units::examples::{ExampleFile, plan as plan_example};

/// The provider requests of `planned`, in order, without local metadata.
fn uploaded(planned: &[Planned]) -> Vec<Value> {
    planned
        .iter()
        .map(|p| crate::requests::provider_request(&p.request).into_owned())
        .collect()
}

/// The requests `rules test` sends for the example `text` at `path` of the
/// only question `toml` defines.
fn example_requests(options: &CheckArgs, path: &str, text: &str) -> Result<Vec<Value>> {
    let budget = TokenBudget::default();
    let file = ExampleFile {
        owner: 0,
        path: std::path::Path::new(path),
        text,
    };
    let plan = plan_example(
        &options.questions[0],
        &file,
        (options.model(), budget.uncached()),
    )?;
    Ok(uploaded(&plan.requests))
}

#[test]
fn an_example_is_asked_as_a_check_asks_its_file_with_only_its_question() {
    let functions = format!(
        "{}{}fn tiny() -> i32 {{\n    1\n}}\n",
        function("charge"),
        long_function("refund")
    );
    let cases = [
        ("function", "src/orders.rs", functions.as_str()),
        (
            "comment",
            "src/lib.rs",
            "/// Sums the values.\nfn total(values: &[i32]) -> i32 {\n    // TODO: handle overflow\n    values.iter().sum()\n}\n",
        ),
        (
            "test",
            "tests/api.rs",
            "#[test]\nfn totals() {\n    let values = [1, 2];\n    assert_eq!(values.iter().sum::<i32>(), 3);\n}\n",
        ),
        (
            "section",
            "AGENTS.md",
            "# Agents\n\n## Checks\n\nRun `cargo test` before every commit.\n\n## Style\n\nKeep functions short.\n",
        ),
        (
            "file",
            "scripts/deploy.sh",
            "#!/bin/sh\nrsync -a dist/ host:/srv\n",
        ),
        // A preview language, and a `.h` header read as C++ by its code.
        (
            "function",
            "src/Shop.kt",
            "fun total(values: List<Int>): Int {\n    var sum = 0\n    for (value in values) {\n        sum += value\n    }\n    return sum * 2 + 1\n}\n",
        ),
        (
            "function",
            "include/cart.h",
            "#include <vector>\n\nnamespace shop {\ninline int total(const std::vector<int>& values) {\n  int sum = 0;\n  for (int value : values) {\n    sum += value;\n  }\n  return sum * 2 + 1;\n}\n}\n",
        ),
    ];
    for (unit, path, text) in cases {
        let toml = format!(
            "[[question]]\nid = \"asked\"\nquestion = \"Does it break the rule?\"\nguidance = \"Say yes only when it does.\"\nunit = \"{unit}\"\npaths = [\"{path}\"]\n"
        );
        let mut options = configured(&toml, &["custom"]);
        options.include_tests = true;
        let project = Project::new();
        project.write(path, text);
        let (_, checked) = planned(&project, &options);
        assert!(!checked.requests.is_empty(), "{unit}: a check asks it");
        assert_eq!(
            example_requests(&options, path, text).unwrap(),
            uploaded(&checked.requests),
            "{unit}"
        );
    }
}

#[test]
fn a_hunk_example_is_the_diff_a_check_sends_with_or_without_its_header() {
    let toml = "[[question]]\nid = \"no-unwrap\"\nquestion = \"Does this change add an unwrap?\"\nunit = \"hunk\"\n";
    let project = Project::new();
    let body: String = (1..=12).map(|n| format!("    let v{n} = {n};\n")).collect();
    project.write("lib.rs", &format!("fn long() {{\n{body}}}\n"));
    git(&project, &["init", "-q"]);
    git(&project, &["add", "."]);
    git(&project, &["commit", "-qm", "base"]);
    let edited = body.replace("let v6 = 6;", "let v6 = find().unwrap();");
    project.write("lib.rs", &format!("fn long() {{\n{edited}}}\n"));
    let mut options = configured(toml, &["custom"]);
    options.base = Some(crate::revision::resolve(&project.0, "HEAD").unwrap());
    let (_, checked) = planned(&project, &options);
    let diff = crate::tests::git::run(
        &project.0,
        &["diff", "--no-color", "--unified=3", "HEAD", "--", "lib.rs"],
    );
    assert_eq!(
        example_requests(&options, "lib.rs", &diff).unwrap(),
        uploaded(&checked.requests),
        "git's own diff as the example"
    );
    let written =
        "     let v5 = 5;\n-    let v6 = 6;\n+    let v6 = find().unwrap();\n\n     let v7 = 7;\n";
    let asked = example_requests(&options, "lib.rs", written).unwrap();
    let hunk = &asked[0]["state"]["hunks"][0];
    assert_eq!(hunk["lines"], "2", "no header: the change starts at line 1");
    assert_eq!(
        hunk["diff"],
        "     let v5 = 5;\n-    let v6 = 6;\n+    let v6 = find().unwrap();\n \n     let v7 = 7;",
        "an empty line is an unchanged blank line"
    );
    assert!(hunk.get("in").is_none());
}

#[test]
fn an_example_without_a_unit_to_ask_is_an_error_saying_what_it_lacks() {
    for (unit, path, text, problem) in [
        (
            "function",
            "src/lib.rs",
            "const LIMIT: u32 = 3;\n",
            "it holds no function",
        ),
        (
            "function",
            "main.zig",
            "pub fn main() void {}\n",
            "JevGate has no Zig parser, and a function question needs one",
        ),
        (
            "function",
            "src/lib.rs",
            "fn broken( {\n",
            "the Rust parser could not read it",
        ),
        (
            "test",
            "src/lib.rs",
            "fn helper() {}\n",
            "it holds no test case",
        ),
        (
            "section",
            "AGENTS.md",
            "# Agents\n",
            "it holds no heading section with text",
        ),
        (
            "hunk",
            "src/lib.rs",
            " unchanged();\n",
            "it holds no changed line: start added lines with `+`",
        ),
    ] {
        let toml =
            format!("[[question]]\nid = \"asked\"\nquestion = \"Is it?\"\nunit = \"{unit}\"\n");
        let options = configured(&toml, &["custom"]);
        let error = format!("{:#}", example_requests(&options, path, text).unwrap_err());
        assert!(
            error.to_lowercase().contains(&problem.to_lowercase()),
            "{unit} {path}: {error}"
        );
    }
}
