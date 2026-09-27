//! Bend 2 laws: which claims are asked, what they are asked with, and the
//! finding a comment that claims more than its law raises.
use super::*;

const MAIN: &str = "import Base\n\n# the response for a page: the status line, the headers, a blank line, then the page\ndef http_response(page: String) -> String:\n  \"HTTP/1.1 200 OK\\r\\n\\r\\n\" ++ page\n\ndef Sorted(xs: List<&2, Nat>) -> Type:\n  Unit\n\ndef sort(xs: List<&2, Nat>) -> List<&2, Nat>:\n  xs\n";

const LAWS: &str = "# The laws of the server: they pin http_response, its pure part.\nimport Base\nimport ./main.bend as Srv\n\n# LAW: the response of any page is some head, then a blank line, then\n# the page: a client that cuts at the blank line reads the page back.\nlaw page_after_blank:\n  for +page: String\n  exs head: String\n  {head ++ (\"\\r\\n\\r\\n\" ++ page) == Srv.http_response(page) : String}\n\n# LAW: the output of sort is sorted\nlaw sort_sorted:\n  for +xs: List<&2, Nat>\n  Srv.Sorted(Srv.sort(xs))\n\n# LAW: a page of two lines has one blank line (sanity)\nlaw one_blank:\n  {Srv.http_response(\"a\") == \"HTTP/1.1 200 OK\\r\\n\\r\\na\" : String}\n\n# Signatures\n# ----------\nlaw main:\n  IO(Unit)\n\nlaw uncommented:\n  for +page: String\n  {Srv.http_response(page) == Srv.http_response(page) : String}\n";

const PROOF: &str = "import Base\nimport ./LAWS.bend as Laws\n\n# the head is the status line\nlaw head_is_status:\n  for +page: String\n  {Laws.Srv.http_response(page) == Laws.Srv.http_response(page) : String}\n";

fn laws_project() -> (Project, CheckArgs) {
    project_with(
        &[
            ("server/main.bend", MAIN),
            ("server/LAWS.bend", LAWS),
            ("server/PROOF.bend", PROOF),
        ],
        &[catalog::LAWS],
    )
}

#[test]
fn claims_that_quantify_under_a_comment_are_asked_with_their_reading_and_defs() {
    let (project, options) = laws_project();
    let (_, plan) = planned(&project, &options);
    assert_eq!(stages(&plan), ["laws"]);
    let request = &plan.requests[0].request;
    let laws = request["state"]["laws"].as_array().unwrap();
    let names: Vec<&str> = laws.iter().map(|l| l["name"].as_str().unwrap()).collect();
    // Not the spot check, the signature, the law without a comment nor the
    // lemma of PROOF.bend.
    assert_eq!(names, ["page_after_blank", "sort_sorted"]);
    assert_eq!(
        laws[0]["reading"],
        "for every page: String, there is some head: String such that head ++ (\"\\r\\n\\r\\n\" ++ page) == Srv.http_response(page)."
    );
    assert_eq!(
        laws[1]["reading"], "for every xs: List<&2, Nat>: Srv.Sorted(Srv.sort(xs)) holds.",
        "Sorted computes a type in another file, so the law is a claim"
    );
    assert!(
        laws[0]["comment"]
            .as_str()
            .unwrap()
            .starts_with("# LAW: the response")
    );
    assert_eq!(laws[0]["defs"][0]["name"], "http_response");
    assert!(
        laws[0]["defs"][0]["source"]
            .as_str()
            .unwrap()
            .contains("++ page")
    );
    assert_eq!(
        request["state"]["file"]["comment"],
        "# The laws of the server: they pin http_response, its pure part."
    );
    assert_eq!(request["state"]["file"]["language"], "Bend 2");
    assert!(request["state"]["file"]["notation"].is_string());
    let note = request["questions"]["l0_states"]["instructions"]["note"]
        .as_str()
        .unwrap();
    assert!(note.starts_with("`file.notation` explains"), "{note}");
}

#[test]
fn a_comment_claiming_more_than_its_law_is_a_finding_at_the_law() {
    let (project, options) = laws_project();
    let mut eval = scripted(0);
    eval.overrides = vec![("l0_states", spread(0.05, 0.05, 0.9))];
    let report = run(&project, &options, &mut eval);
    let (dimension, findings) = dimension_of(report, "server/LAWS.bend", catalog::LAWS);
    assert_eq!((dimension.units.judged, dimension.units.review), (2, 1));
    let finding = &findings[0];
    assert_eq!(finding.rule, "tests/laws");
    assert_eq!(finding.line, 7, "the law, below its comment");
    assert!(
        finding
            .message
            .starts_with("The comment above law `page_after_blank` promises more"),
        "{}",
        finding.message
    );
    // "Barely" is a note.
    let mut options = options;
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides = vec![("l0_states", spread(0.1, 0.8, 0.1))];
    let report = run(&project, &options, &mut eval);
    let (dimension, _) = dimension_of(report, "server/LAWS.bend", catalog::LAWS);
    assert_eq!((dimension.units.note, dimension.units.clear), (1, 1));
}

#[test]
fn an_undecided_law_is_asked_what_its_comment_adds() {
    let (project, options) = laws_project();
    let options_of = ["nothing", "context", "property", "inputs", "condition"];
    let mut eval = scripted(0);
    eval.overrides = vec![("l0_states", spread(0.3, 0.3, 0.4))];
    eval.recheck_overrides = vec![("relation", choice_of("property", &options_of))];
    let report = run(&project, &options, &mut eval);
    let (dimension, findings) = dimension_of(report, "server/LAWS.bend", catalog::LAWS);
    assert_eq!(
        (dimension.units.consider, dimension.units.uncertain),
        (1, 0)
    );
    assert!(
        findings[0]
            .message
            .contains("it claims a property the law does not state"),
        "{}",
        findings[0].message
    );
    // A recheck that says the comment only restates the law clears it.
    let mut options = options;
    options.refresh = true;
    let mut eval = scripted(0);
    eval.overrides = vec![("l0_states", spread(0.3, 0.3, 0.4))];
    eval.recheck_overrides = vec![("relation", choice_of("nothing", &options_of))];
    let report = run(&project, &options, &mut eval);
    let (dimension, _) = dimension_of(report, "server/LAWS.bend", catalog::LAWS);
    assert_eq!((dimension.units.clear, dimension.units.uncertain), (2, 0));
}

#[test]
fn a_comment_heading_several_laws_is_asked_with_all_of_them() {
    let laws = "import Base\nimport ./main.bend as Srv\n\n# LAW: sort is sound and complete: it sorts, and it keeps every element\nlaw sort_sorted:\n  for +xs: List<&2, Nat>\n  Srv.Sorted(Srv.sort(xs))\n\nlaw sort_keeps:\n  for +xs: List<&2, Nat>\n  {Srv.sort(xs) == Srv.sort(xs) : List<&2, Nat>}\n\n# LAW: the response ends in the page\nlaw ends_in_page:\n  for +page: String\n  exs head: String\n  {head ++ page == Srv.http_response(page) : String}\n";
    let (project, options) = project_with(
        &[("server/main.bend", MAIN), ("server/LAWS.bend", laws)],
        &[catalog::LAWS],
    );
    let (_, plan) = planned(&project, &options);
    let asked = &plan.requests[0].request["state"]["laws"];
    let names: Vec<&str> = asked
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["sort_sorted", "ends_in_page"]);
    let reading = asked[0]["reading"].as_str().unwrap();
    assert!(
        reading.starts_with("`sort_sorted`: for every xs")
            && reading.contains(" `sort_keeps`: for every xs"),
        "{reading}"
    );
    assert!(
        asked[0]["source"]
            .as_str()
            .unwrap()
            .contains("law sort_keeps:")
    );
    assert!(!asked[1]["reading"].as_str().unwrap().starts_with('`'));
}
