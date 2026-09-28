//! `--base` judging what a change touched: which units are asked and
//! reported, and that `--whole-files` asks what a check of the files asks.
use super::*;

/// A check of what changed since `base`, for `rules`.
fn since(base: &str, rules: &[&str]) -> CheckArgs {
    let mut options = args();
    options.rules = rules.iter().map(|r| r.to_string()).collect();
    options.base = Some(base.into());
    options
}

/// The function names each planned request of `stage` holds.
fn packed(plan: &Plan, stage: &str) -> Vec<Vec<String>> {
    plan.requests
        .iter()
        .filter(|p| p.request["jevgate"]["stage"] == stage)
        .map(|p| {
            p.request["state"]["functions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| f["name"].as_str().unwrap().to_string())
                .collect()
        })
        .collect()
}

/// `lib.rs` with `names` as judged functions, `edited` with a new last line.
fn functions_file(names: &[&str], edited: &[&str]) -> String {
    names
        .iter()
        .map(|name| {
            let text = function(name);
            if edited.contains(name) {
                text.replace("doubled + 1", "doubled + 2")
            } else {
                text
            }
        })
        .collect()
}

const NAMES: [&str; 6] = ["f0", "f1", "f2", "f3", "f4", "f5"];

#[test]
fn only_the_functions_a_change_touched_are_asked_and_reported() {
    let project = Project::new();
    project.write("lib.rs", &functions_file(&NAMES, &[]));
    project.commit_all();
    project.write("lib.rs", &functions_file(&NAMES, &["f2"]));
    let options = since("HEAD", &[catalog::FUNCTION_SIMPLIFICATION]);
    let (inputs, plan) = planned(&project, &options);
    assert!(inputs[0].changed.is_some());
    assert_eq!(packed(&plan, "functions"), [["f2"]]);
    assert!(plan.files[&0].units.iter().all(|u| u.name == "f2"));
    let report = run(&project, &options, &mut scripted(2));
    assert_eq!(report.scope, crate::schema::Scope::ChangedLines);
    let symbols: Vec<_> = report.files[0]
        .findings
        .iter()
        .map(|f| f.symbol.as_deref())
        .collect();
    assert_eq!(symbols, [Some("f2")]);
    assert!(crate::output::headline(&report).contains("· changed lines since "));
}

#[test]
fn a_change_names_only_the_code_it_touched_that_the_parser_could_not_read() {
    // tree-sitter-rust reads snapbox's `str![…]` as the type `str`, so `f3`
    // is left out wherever the change falls.
    let file = |edited: &[&str]| {
        functions_file(&NAMES, edited).replacen(
            "fn f3(values: &[i32]) -> i32 {\n    let mut total = 0;",
            "fn f3(values: &[i32]) -> i32 {\n    let mut total = str![[\"x\"]].len() as i32;",
            1,
        )
    };
    let project = Project::new();
    project.write("lib.rs", &file(&[]));
    project.commit_all();
    project.write("lib.rs", &file(&["f5"]));
    let options = since("HEAD", &[catalog::FUNCTION_SIMPLIFICATION]);
    let (_, plan) = planned(&project, &options);
    assert_eq!(packed(&plan, "functions"), [["f5"]]);
    assert!(
        plan.files[&0].left_out.is_empty(),
        "{:?}",
        plan.files[&0].left_out
    );
    project.write("lib.rs", &file(&["f3", "f5"]));
    let report = run(&project, &options, &mut scripted(0));
    let left_out: Vec<&str> = report.files[0]
        .left_out
        .iter()
        .map(|l| l.unit.as_str())
        .collect();
    assert_eq!(left_out, ["f3"]);
    let mut out = Vec::new();
    crate::output::agent(&mut out, &report, false, crate::output::Style::PLAIN).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("\nLeft out where the parser could not read the code the change touched, the rest of it judged: 1 unit in 1 file.\n  lib.rs:25 f3: The Rust parser could not read line 26.\n"),
        "{text}"
    );
}

#[test]
fn lines_removed_inside_a_function_touch_it() {
    let project = Project::new();
    let source = format!("{}{}", function("f0"), long_function("f1"));
    project.write("lib.rs", &source);
    project.commit_all();
    project.write(
        "lib.rs",
        &source.replace("    let spread = largest - smallest;\n", ""),
    );
    let options = since("HEAD", &[catalog::FUNCTION_SIMPLIFICATION]);
    let (_, plan) = planned(&project, &options);
    assert_eq!(packed(&plan, "functions"), [["f1"]]);
}

#[test]
fn changed_functions_share_a_pack_only_within_a_run_of_the_whole_file() {
    // `count` ends a run: `total` and `other` fall in different runs.
    let names = ["total", "count", "other", "alpha"];
    let project = Project::new();
    project.write("lib.rs", &functions_file(&names, &[]));
    project.commit_all();
    project.write("lib.rs", &functions_file(&names, &["total", "other"]));
    let options = since("HEAD", &[catalog::FUNCTION_SIMPLIFICATION]);
    let (_, plan) = planned(&project, &options);
    assert_eq!(packed(&plan, "functions"), [["total"], ["other"]]);
    project.write(
        "lib.rs",
        &functions_file(&names, &["total", "other", "alpha"]),
    );
    let (_, plan) = planned(&project, &options);
    assert_eq!(
        packed(&plan, "functions"),
        [vec!["total"], vec!["other", "alpha"]]
    );
}

#[test]
fn a_new_file_is_judged_whole() {
    let project = Project::new();
    project.write("lib.rs", &functions_file(&NAMES[..2], &[]));
    project.commit_all();
    project.write("new.rs", &functions_file(&NAMES[2..4], &[]));
    let options = since("HEAD", &[catalog::FUNCTION_SIMPLIFICATION]);
    let (inputs, plan) = planned(&project, &options);
    assert_eq!(inputs.len(), 1);
    assert!(inputs[0].changed.is_none());
    assert_eq!(packed(&plan, "functions").concat(), ["f2", "f3"]);
}

#[test]
fn whole_files_asks_what_a_check_of_the_changed_files_asks() {
    let project = Project::new();
    project.write("lib.rs", &functions_file(&NAMES, &[]));
    project.commit_all();
    project.write("lib.rs", &functions_file(&NAMES, &["f2"]));
    let bodies = |options: &CheckArgs| -> Vec<String> {
        let (_, plan) = planned(&project, options);
        plan.requests
            .iter()
            .map(|p| p.request.to_string())
            .collect()
    };
    let rules = [catalog::FUNCTION_SIMPLIFICATION];
    let mut whole = since("HEAD", &rules);
    whole.whole_files = true;
    let mut files = since("HEAD", &rules);
    files.base = None;
    // Runs `f0`-`f2`, `f3`-`f4` and `f5`: three packs, of which the change
    // touched one function.
    assert_eq!(bodies(&whole), bodies(&files));
    assert_eq!(bodies(&files).len(), 3);
    assert_eq!(bodies(&since("HEAD", &rules)).len(), 1);
}

#[test]
fn a_copy_pair_is_asked_when_either_copy_changed() {
    let team = super::duplicates::LOAD
        .replace("load_user", "load_team")
        .replace("\"name\"", "\"title\"");
    let file = |copy: &str, tail: &str, edited: bool| {
        let text = format!("{copy}\n{}", function(tail));
        if edited {
            text.replace("doubled + 1", "doubled + 2")
        } else {
            text
        }
    };
    let project = Project::new();
    project.write("a.rs", &file(super::duplicates::LOAD, "tail_a", false));
    project.write("b.rs", &file(&team, "tail_b", false));
    project.commit_all();
    // Both files change beside their copies: the pair is not asked.
    project.write("a.rs", &file(super::duplicates::LOAD, "tail_a", true));
    project.write("b.rs", &file(&team, "tail_b", true));
    let options = since("HEAD", &[catalog::SHARED_LOGIC]);
    let (_, plan) = planned(&project, &options);
    assert!(stages(&plan).is_empty(), "{:?}", stages(&plan));
    // One copy changes: the pair is asked, by the file that owns it.
    let changed = team.replace("anonymous", "nobody");
    project.write("b.rs", &file(&changed, "tail_b", true));
    let (_, plan) = planned(&project, &options);
    assert_eq!(stages(&plan), ["duplicate-pair"]);
    assert_eq!(file_plan(&plan, "a.rs").units.len(), 1);
}

#[test]
fn only_the_constants_and_comments_a_change_touched_are_asked() {
    let source = format!(
        "const REGION: &str = \"eu-west-1\";\nconst ZONE: &str = \"zone-a\";\n\n// Adds the values twice over.\n{}// Sums the values and adds one.\n{}",
        function("f0"),
        function("f1")
    );
    let project = Project::new();
    project.write("lib.rs", &source);
    project.commit_all();
    project.write(
        "lib.rs",
        &source
            .replace("zone-a", "zone-b")
            .replace("and adds one", "then adds one"),
    );
    let options = since("HEAD", &[catalog::HARDCODED_VALUES, catalog::COMMENTS]);
    let (_, plan) = planned(&project, &options);
    assert_eq!(stages(&plan), ["constants", "comments"]);
    let constants = &first_request(&plan, "constants")["state"]["constants"];
    assert_eq!(constants.as_array().unwrap().len(), 1);
    assert_eq!(constants[0]["name"], "ZONE");
    let comments = &first_request(&plan, "comments")["state"]["comments"];
    assert_eq!(comments.as_array().unwrap().len(), 1);
    assert!(
        comments[0]["text"]
            .as_str()
            .unwrap()
            .contains("then adds one")
    );
}

#[test]
fn a_document_the_change_left_alone_is_checked_for_the_paths_it_removed() {
    let project = Project::new();
    project.write("src/old.rs", &function("old"));
    project.write("src/kept.rs", &function("kept"));
    project.write(
        "README.md",
        "# Setup\nRun `src/old.rs` to start.\n\n# Other\nSee `docs/missing.md` first.\n",
    );
    project.write("docs/guide.md", "# Guide\nRead `src/kept.rs`.\n");
    project.commit_all();
    project.git(&["rm", "-q", "src/old.rs"]);
    project.git(&["commit", "-qm", "remove"]);
    let options = since("HEAD~1", &[catalog::DOC_STALENESS]);
    let (inputs, plan) = planned(&project, &options);
    let paths: Vec<_> = inputs.iter().map(|i| i.result.path.clone()).collect();
    assert_eq!(paths, [std::path::PathBuf::from("README.md")]);
    // Its finished-plan question, since a path it names was removed, and
    // only the section naming that path.
    let units: Vec<&str> = file_plan(&plan, "README.md")
        .units
        .iter()
        .map(|u| u.name.as_str())
        .collect();
    assert_eq!(units, ["README.md", "Setup"]);
    let mut not_a_plan = scripted(2);
    not_a_plan.overrides.push(("plan", noul_at(0.05)));
    let report = run(&project, &options, &mut not_a_plan);
    let messages: Vec<&str> = report.files[0]
        .findings
        .iter()
        .map(|f| f.message.as_str())
        .collect();
    assert!(
        messages.len() == 1 && messages[0].contains("`src/old.rs`, which was deleted"),
        "{messages:?}"
    );
}

/// The stages a check of `rule` plans for `path`, committed as `base`, after
/// each of `edits`.
fn stages_after(path: &str, base: &str, edits: &[String], rule: &str) -> Vec<Vec<String>> {
    let project = Project::new();
    project.write(path, base);
    project.commit_all();
    let options = since("HEAD", &[rule]);
    edits
        .iter()
        .map(|text| {
            project.write(path, text);
            let (_, plan) = planned(&project, &options);
            stages(&plan).into_iter().map(String::from).collect()
        })
        .collect()
}

/// No stage, then only `stage`: an outline asked only after the edit that
/// adds a member.
fn only_after_adding(stage: &str) -> Vec<Vec<String>> {
    vec![Vec::new(), vec![stage.to_string()]]
}

#[test]
fn a_file_outline_is_asked_only_when_the_change_adds_a_member() {
    let names: Vec<String> = (0..14).map(|i| format!("f{i}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let edited = functions_file(&names, &["f3"]);
    let added = format!("{edited}{}", function("f14"));
    let base = functions_file(&names, &[]);
    let asked = stages_after(
        "lib.rs",
        &base,
        &[edited, added],
        catalog::FILE_ORGANIZATION,
    );
    assert_eq!(asked, only_after_adding("outline"));
}

#[test]
fn a_test_file_outline_is_asked_only_when_the_change_adds_a_test() {
    let case = |i: usize| {
        format!(
            "#[test]\nfn case_{i}() {{\n    let values = [1, 2, 3];\n    let mut total = 0;\n    for value in values {{\n        total += value;\n    }}\n    assert_eq!(total, 6);\n}}\n"
        )
    };
    let cases: String = (0..12).map(case).collect();
    let edited = cases.replacen("total, 6", "total, 3 + 3", 1);
    let added = format!("{cases}{}", case(12));
    let rule = catalog::FILE_ORGANIZATION;
    let asked = stages_after("tests/cases.rs", &cases, &[edited, added], rule);
    assert_eq!(asked, only_after_adding("outline"));
}

#[test]
fn a_long_document_is_asked_about_its_outline_only_when_the_change_adds_a_heading() {
    let text = format!(
        "# Guide\n{}## Install\n{}",
        "Some prose.\n".repeat(160),
        "More prose.\n".repeat(160)
    );
    let edited = text.replacen("Some prose.", "Some other prose.", 1);
    let added = format!("{text}## Upgrade\nSteps.\n");
    let asked = stages_after(
        "docs/guide.md",
        &text,
        &[edited, added],
        catalog::LARGE_DOCS,
    );
    assert_eq!(asked, only_after_adding("docs"));
}

#[test]
fn a_pull_request_check_fails_only_on_what_the_change_touched() {
    // Changed lines and the default gate together: every function answers
    // review-worthy, only the touched one is judged, and its mature
    // function-simplification review alone fails the gate.
    let project = Project::new();
    project.write("lib.rs", &functions_file(&NAMES, &[]));
    project.commit_all();
    project.write("lib.rs", &functions_file(&NAMES, &["f2"]));
    let mut options = since("HEAD", &[catalog::FUNCTION_SIMPLIFICATION]);
    let report = run(&project, &options, &mut scripted(2));
    assert_eq!(report.gate.unwrap().reasons, ["1 new review finding"]);
    assert_eq!(
        report.files[0].findings[0].gate,
        Some(crate::schema::Gating::Fails)
    );
    options.whole_files = true;
    let whole = run(&project, &options, &mut scripted(2));
    assert_eq!(whole.gate.unwrap().reasons, ["6 new review findings"]);
}

const BODY_LOGS: &str = r#"
[[question]]
id = "body-logs"
question = "Does this function write a request body to a log?"
unit = "function"
"#;

#[test]
fn a_custom_question_asks_only_about_the_units_a_change_touched() {
    let project = Project::new();
    project.write("lib.rs", &functions_file(&NAMES, &[]));
    project.commit_all();
    project.write("lib.rs", &functions_file(&NAMES, &["f2"]));
    let mut alone = super::custom::configured(BODY_LOGS, &["custom"]);
    alone.base = Some("HEAD".into());
    let (_, plan) = planned(&project, &alone);
    assert_eq!(packed(&plan, "custom"), [["f2"]]);
    let units: Vec<&str> = plan.files[&0].units.iter().map(|u| &*u.name).collect();
    assert_eq!(units, ["f2"], "the others are neither asked nor counted");
    let mut riding = super::custom::configured(BODY_LOGS, &["default", "custom"]);
    riding.base = Some("HEAD".into());
    let (_, plan) = planned(&project, &riding);
    let custom: Vec<&Planned> = plan
        .requests
        .iter()
        .filter(|p| p.request["questions"].to_string().contains("custom_"))
        .collect();
    assert_eq!(custom.len(), 1, "it rides beside function simplification");
    assert_eq!(custom[0].request["state"]["functions"][0]["name"], "f2");
    alone.whole_files = true;
    let (_, plan) = planned(&project, &alone);
    assert_eq!(packed(&plan, "custom").concat(), NAMES);
}

#[test]
fn a_hunk_question_asks_about_a_change_that_only_removes_lines() {
    let toml = r#"
[[question]]
id = "kept-totals"
question = "Does this change drop a running total?"
unit = "hunk"
"#;
    let project = Project::new();
    project.write("lib.rs", &functions_file(&NAMES, &[]));
    project.commit_all();
    let removed: String = NAMES
        .iter()
        .map(|&name| match name {
            "f2" => function(name).replace("    let mut total = 0;\n", ""),
            _ => function(name),
        })
        .collect();
    project.write("lib.rs", &removed);
    let mut options = super::custom::configured(toml, &["custom"]);
    options.base = Some("HEAD".into());
    let (_, plan) = planned(&project, &options);
    let hunks: Vec<(usize, usize)> = plan.files[&0]
        .units
        .iter()
        .map(|u| (u.locations[0].start_line, u.locations[0].end_line))
        .collect();
    assert_eq!(
        hunks,
        [(18, 18)],
        "a removal sits at the line after it, outside the lines left"
    );
    assert_eq!(plan.requests.len(), 1);
}

/// Asks `toml`'s questions alone about what changed since `HEAD`.
fn custom_since_head(toml: &str) -> CheckArgs {
    let mut options = super::custom::configured(toml, &["custom"]);
    options.base = Some("HEAD".into());
    options
}

/// The names of the units planned for the file whose path ends with `name`.
fn unit_names<'p>(plan: &'p Plan, name: &str) -> Vec<&'p str> {
    file_plan(plan, name)
        .units
        .iter()
        .map(|u| u.name.as_str())
        .collect()
}

const VIEWS: &str = "def home(request):\n    return 1\n\n\n@login_required\ndef orders(request):\n    return 2\n\n\ndef gone():\n    return 3\n\n\ndef listed(request):\n    return 4\n\n\ndef total(values):\n    result = sum(values)\n    audit(result)\n";

#[test]
fn a_custom_question_counts_what_a_removal_at_a_units_edge_took_from_it() {
    let project = Project::new();
    project.write("views.py", VIEWS);
    project.commit_all();
    let edited = VIEWS
        .replace("@login_required\n", "")
        .replace("def gone():\n    return 3\n\n\n", "")
        .replace("    audit(result)\n", "");
    project.write("views.py", &edited);
    let (_, plan) = planned(&project, &custom_since_head(BODY_LOGS));
    assert_eq!(
        unit_names(&plan, "views.py"),
        ["orders", "total"],
        "the decorator above `orders` and the last statement of `total` were theirs; \
         `listed` only lost the function above it"
    );
    let built_in = since("HEAD", &[catalog::FUNCTION_SIMPLIFICATION]);
    let (_, plan) = planned(&project, &built_in);
    assert!(
        file_plan(&plan, "views.py").units.is_empty(),
        "the built-in rules leave a removal at an edge to the code beside it"
    );
}

const LICENSED: &str = "// Copyright 2026 Example\n// SPDX-License-Identifier: MIT\n\npub fn total() -> u32 {\n    1\n}\n";

#[test]
fn a_file_question_asks_about_a_file_the_change_edits_anywhere_or_moves() {
    let toml = r#"
[[question]]
id = "license-header"
question = "Does this file lack its license header?"
unit = "file"
"#;
    let project = Project::new();
    for name in ["head.rs", "old.rs", "same.rs"] {
        project.write(name, LICENSED);
    }
    project.write("tail.rs", &format!("{LICENSED}// End of file.\n"));
    project.commit_all();
    project.write(
        "head.rs",
        &LICENSED.replace(
            "// Copyright 2026 Example\n// SPDX-License-Identifier: MIT\n",
            "",
        ),
    );
    project.write("tail.rs", LICENSED);
    project.git(&["mv", "old.rs", "moved.rs"]);
    project.write("same.rs", LICENSED);
    let (_, plan) = planned(&project, &custom_since_head(toml));
    let asked: Vec<String> = plan
        .files
        .values()
        .filter(|file| !file.units.is_empty())
        .map(|file| file.path.display().to_string())
        .collect();
    assert_eq!(asked, ["head.rs", "moved.rs", "tail.rs"]);
}

#[test]
fn a_file_moved_into_a_questions_paths_is_new_to_it() {
    let toml = r#"
[[question]]
id = "api-body-logs"
question = "Does this function write a request body to a log?"
unit = "function"
paths = ["src/api/**"]
"#;
    let project = Project::new();
    project.write("lib/handler.rs", &functions_file(&["f0", "f1"], &[]));
    project.write("src/api/kept.rs", &functions_file(&["f2"], &[]));
    project.commit_all();
    project.git(&["mv", "lib/handler.rs", "src/api/handler.rs"]);
    project.git(&["mv", "src/api/kept.rs", "src/api/renamed.rs"]);
    let (_, plan) = planned(&project, &custom_since_head(toml));
    assert_eq!(unit_names(&plan, "src/api/handler.rs"), ["f0", "f1"]);
    assert!(
        unit_names(&plan, "src/api/renamed.rs").is_empty(),
        "a file the question already read, moved untouched"
    );
}

#[test]
fn a_hunk_keeps_its_blank_lines_however_git_is_set_to_print_them() {
    let toml = r#"
[[question]]
id = "no-body-logs"
question = "Does this change log a request body?"
unit = "hunk"
"#;
    let project = Project::new();
    project.write(
        "m.py",
        "def charge(order):\n    x = 1\n\n    y = 2\n    return x + y\n",
    );
    project.commit_all();
    project.git(&["config", "diff.suppressBlankEmpty", "true"]);
    project.write(
        "m.py",
        "def charge(order):\n    x = 2\n\n    y = log(order.body)\n    return x + y\n",
    );
    let (_, plan) = planned(&project, &custom_since_head(toml));
    assert_eq!(unit_names(&plan, "m.py"), ["lines 2–4"]);
    let diff = &plan.requests[0].request["state"]["hunks"][0]["diff"];
    assert!(
        diff.as_str().unwrap().contains("+    y = log(order.body)"),
        "{diff}"
    );
}
