//! The candidate lines of an instruction file: list items and paragraphs,
//! with what introduces them, and what is left out.
use super::AGENTS;
use crate::custom::propose::lines::{Line, lines};

/// Each candidate's first line, text and lead-in.
fn texts(found: &[Line]) -> Vec<(usize, &str, Option<&str>)> {
    found
        .iter()
        .map(|l| (l.start_line, l.text.as_str(), l.lead_in.as_deref()))
        .collect()
}

#[test]
fn candidates_are_list_items_at_any_depth_and_paragraphs_with_what_introduces_them() {
    let found = lines(AGENTS);
    let rules = Some("Handlers follow these rules:");
    assert_eq!(
        texts(&found),
        [
            (3, "Prefer `undefined` for absent values.", None),
            (7, "Never log request bodies.", rules),
            (8, "Run `cargo test` before pushing.", rules),
            (
                10,
                "Never swallow an error without a log line.",
                Some("**Errors**:")
            ),
            (
                11,
                "Wrap errors with context about the call that failed.",
                Some("**Errors**:")
            ),
        ],
        "introductions, tables, code, comments and imports are no candidates"
    );
    assert!(found.iter().all(|l| l.heading == "Conventions"));
    assert_eq!(
        found[4].end_line, 12,
        "a continuation line belongs to its item"
    );
}

#[test]
fn a_line_ending_in_a_colon_is_a_candidate_when_no_list_follows_it() {
    let found = lines("# A\n\n- Log with context:\n- Next item\n\nRun this:\n\n```sh\nmake\n```\n");
    assert_eq!(
        texts(&found),
        [
            (3, "Log with context:", None),
            (4, "Next item", None),
            (6, "Run this:", None),
        ]
    );
}

#[test]
fn a_paragraph_at_the_margin_ends_the_list_and_its_introduction() {
    let found = lines("Rules:\n- One\n\n  Still one.\n\nAfter the list.\n- Two\n");
    assert_eq!(
        texts(&found),
        [
            (2, "One", Some("Rules:")),
            (4, "Still one.", Some("One")),
            (6, "After the list.", None),
            (7, "Two", None),
        ]
    );
}

#[test]
fn frontmatter_task_boxes_quotes_and_numbered_items_are_read_as_text() {
    let source = "---\nglobs: src/**\n---\n1. First rule\n2. [x] Done rule\n> Quoted rule\n> on two lines\n* * *\nPlain text rule\n";
    assert_eq!(
        texts(&lines(source)),
        [
            (4, "First rule", None),
            (5, "Done rule", None),
            (6, "Quoted rule on two lines", None),
            (9, "Plain text rule", None),
        ]
    );
}

#[test]
fn control_and_bidirectional_characters_are_removed() {
    let found = lines("- Never \u{1b}[31mlog\u{202e} bodies\u{7}\n");
    assert_eq!(found[0].text, "Never [31mlog bodies");
}

#[test]
fn a_comment_keeps_the_line_numbers_below_it() {
    let found = lines("<!--\nhidden\n-->\n- Never log bodies\n");
    assert_eq!(texts(&found), [(4, "Never log bodies", None)]);
}

#[test]
fn a_byte_order_mark_hides_no_heading_or_frontmatter() {
    let found = lines("\u{feff}# Rules\n\n- Never log bodies\n");
    assert_eq!(texts(&found), [(3, "Never log bodies", None)]);
    assert_eq!(found[0].heading, "Rules");
    let found = lines("\u{feff}---\nglobs: src/**\n---\n- Never log bodies\n");
    assert_eq!(texts(&found), [(4, "Never log bodies", None)]);
}
