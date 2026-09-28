//! Partial parses: a syntax error leaves out the unit it sits in, named in
//! the plan, and the rest of its file is judged.
use super::*;
use crate::{analysis::test_map, schema::LeftOut};

/// A function tree-sitter-rust misreads, large enough to judge: it takes
/// snapbox's `str![…]` for the type `str`, one error on its sixth line.
fn misread(name: &str) -> String {
    function(name).replace(
        "let doubled = total * 2;",
        "let doubled = str![[\"x\"]].len() as i32 * total;",
    )
}

/// `function` with a comment above its first statement.
fn commented(function: String) -> String {
    function.replacen(
        "    let mut total = 0;\n",
        "    // Sum first and double after: callers expect an odd total.\n    let mut total = 0;\n",
        1,
    )
}

#[test]
fn a_syntax_error_leaves_out_its_function_and_the_rest_of_the_file_is_judged() {
    let source = format!(
        "{}{}{}",
        function("first"),
        misread("broken"),
        function("last")
    );
    let (project, options) = rule_project(&source, catalog::FUNCTION_SIMPLIFICATION);
    let (_, plan) = planned(&project, &options);
    let file = file_plan(&plan, "lib.rs");
    let judged: Vec<&str> = file.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(judged, ["first", "last"]);
    let sent = first_request(&plan, "functions").to_string();
    assert!(sent.contains("fn first") && !sent.contains("fn broken"));
    assert_eq!(
        file.left_out,
        [LeftOut {
            unit: "broken".into(),
            start_line: 9,
            end_line: 16,
            reason: "Syntax error at line 14.".into(),
        }]
    );
}

#[test]
fn a_comment_in_a_left_out_function_is_left_out_with_it() {
    let source = format!(
        "{}{}",
        commented(function("kept")),
        commented(misread("broken"))
    );
    let (project, options) = rule_project(&source, catalog::COMMENTS);
    let (_, plan) = planned(&project, &options);
    let request = first_request(&plan, "comments");
    let comments = request["state"]["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert_eq!(comments[0]["in"], "fn kept(values: &[i32]) -> i32");
}

#[test]
fn a_file_whose_every_unit_is_left_out_is_skipped_whole() {
    let source = format!("{}{}", misread("a"), misread("b"));
    let (project, options) = rule_project(&source, catalog::FUNCTION_SIMPLIFICATION);
    let (inputs, plan) = planned(&project, &options);
    let owner = inputs
        .iter()
        .position(|i| i.result.path.ends_with("lib.rs"))
        .unwrap();
    assert!(!plan.files.contains_key(&owner));
    assert_eq!(plan.skipped[&owner], crate::syntax::SYNTAX_ERRORS);
}

/// Sixteen functions of each of two concerns, `broken` of them misread.
fn two_concerns_misread(broken: usize) -> String {
    let mut source = String::from("struct Cache { entries: Vec<u8> }\n");
    for i in 0..16 {
        let name = format!("warm{i}");
        source.push_str(&if i < broken {
            misread(&name)
        } else {
            function(&name)
        });
    }
    source.push_str("struct Page { body: String }\n");
    for i in 0..16 {
        source.push_str(&function(&format!("render{i}")));
    }
    source
}

#[test]
fn an_outline_needs_the_parse_to_cover_most_of_the_file() {
    let outline = |broken: usize| {
        let (project, options) = project_with(
            &[("lib.rs", &two_concerns_misread(broken))],
            &[catalog::FILE_ORGANIZATION, catalog::FUNCTION_SIMPLIFICATION],
        );
        let (_, plan) = planned(&project, &options);
        let file = file_plan(&plan, "lib.rs").clone();
        let outlines = file
            .units
            .iter()
            .filter(|u| u.rule == catalog::FILE_ORGANIZATION)
            .count();
        (outlines, file)
    };
    // One function of 32 left out: the outline lists the other 31 members.
    let (outlines, file) = outline(1);
    assert_eq!(outlines, 1);
    assert_eq!(file.left_out.len(), 1);
    // Half the file left out: an outline would describe another file.
    let (outlines, file) = outline(16);
    assert_eq!(outlines, 0);
    let last = file.left_out.last().unwrap();
    assert_eq!(
        (last.unit.as_str(), last.start_line, last.end_line),
        ("outline", 1, 258)
    );
    assert!(
        last.reason
            .starts_with("Only 50% of the file's lines parsed"),
        "{}",
        last.reason
    );
}

#[test]
fn a_test_holding_a_syntax_error_is_left_out_and_named() {
    let script = "import { total } from './total';\ndescribe('total', () => {\n  it('adds', () => {\n    expect(total([1, 2])).toBe(3);\n  });\n  it('parses', () => {\n    expect(total([1,, @])).toBe(1);\n  });\n});\n";
    let (project, options) = tests_project(
        &[
            (
                "total.ts",
                "export const total = (xs: number[]) => xs.length;\n",
            ),
            ("total.test.ts", script),
        ],
        catalog::TEST_VALUE,
    );
    let (_, plan) = planned(&project, &options);
    let file = file_plan(&plan, "total.test.ts");
    let tests: Vec<&str> = file.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(tests, ["adds"]);
    assert_eq!(
        file.left_out,
        [LeftOut {
            unit: "parses".into(),
            start_line: 6,
            end_line: 8,
            reason: "Syntax error at line 7.".into(),
        }]
    );
    assert_eq!(
        test_map::broken_cases(std::path::Path::new("total.test.ts"), script)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_generic_function_holding_an_error_is_left_out_with_its_comments() {
    let source = "fun first(x: Int): Int {\n    // Count from one: the grid's first row is its header.\n    return x + 1\n}\n\nfun broken(x: Int): Int {\n    // Double it: the grid is twice as wide as it is tall.\n    val y: = x\n    return y\n}\n";
    let (project, options) = project_with(&[("Grid.kt", source)], &[catalog::COMMENTS]);
    let (_, plan) = planned(&project, &options);
    assert_eq!(
        file_plan(&plan, "Grid.kt").left_out,
        [LeftOut {
            unit: "broken".into(),
            start_line: 6,
            end_line: 10,
            reason: "Syntax error at line 8.".into(),
        }]
    );
    let request = first_request(&plan, "comments");
    let comments = request["state"]["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert_eq!(comments[0]["in"], "fun first(x: Int): Int");
}
