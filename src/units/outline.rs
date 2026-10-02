//! File organization: an outline of member signatures, sizes and groups,
//! without bodies, asked one look-here question: whether the file does
//! several separate kinds of work a maintainer could keep apart. A test
//! file's members are its test cases and the support code they share. The
//! split Score, the kind of file and its candidate parts once decided these
//! findings; on files labeled for splitting, the look-here question flagged
//! 6 of 9 against 5 of 54 kept.
use super::{
    Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan, identity, outcome::LOOK,
    questions,
};
use crate::{
    analysis::{
        groups,
        test_map::TestCase,
        units::{FileUnits, Kind, Unit},
    },
    catalog::FILE_ORGANIZATION,
    schema::Pass,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

const CALLS: usize = 12;
const USED_BY: usize = 3;
/// Files with fewer non-blank lines are too small to split.
pub const MIN_FILE_LINES: usize = 100;
/// The same for Bend 2, which writes each match arm, binding and effect on
/// a line of its own. On 25 Bend 2 projects, the 16 file-organization
/// findings on files with fewer member lines were all labeled wrong, and
/// the 13 right ones were on files of 313 member lines or more.
pub const MIN_BEND_FILE_LINES: usize = 300;

/// One listed member: its name, its lines and the state sent for it.
struct Member {
    name: String,
    line: usize,
    end_line: usize,
    state: Value,
}

/// An application file's members outside its tests.
pub(super) fn plan(
    file: &FileContext<'_>,
    parsed: &FileUnits,
    members: &[usize],
    callers: &BTreeMap<String, BTreeSet<PathBuf>>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let units = &parsed.units;
    let position: BTreeMap<usize, usize> =
        members.iter().enumerate().map(|(p, &m)| (m, p)).collect();
    let sets = groups::groups(units, members, &parsed.imports)
        .into_iter()
        .map(|g| g.members.iter().map(|m| position[m]).collect())
        .collect();
    let listed = member_state(file, units, members, callers);
    plan_outline(file, false, listed, sets, out, requests);
}

/// A test file's cases, with their suites and subjects, and the helpers,
/// fixtures and types outside them.
pub(super) fn plan_tests(
    file: &FileContext<'_>,
    parsed: &FileUnits,
    cases: &[TestCase],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let support: Vec<&Unit> = parsed
        .units
        .iter()
        .filter(|u| !cases.iter().any(|c| u.overlaps(&(c.line..c.end_line + 1))))
        .collect();
    if cases.len() + support.len() < 2 {
        return;
    }
    let helpers: BTreeSet<&str> = support.iter().map(|u| u.short_name.as_str()).collect();
    let listed = cases
        .iter()
        .map(|case| {
            let mut state = json!({
                "name": case.name,
                "kind": "test",
                "lines": case.end_line + 1 - case.line,
            });
            if !case.suite.is_empty() {
                state["suite"] = json!(case.suite.join(" > "));
            }
            if !case.subjects.is_empty() {
                state["subjects"] = json!(case.subjects.iter().take(CALLS).collect::<Vec<_>>());
            }
            let calls: Vec<&String> = case
                .calls
                .iter()
                .filter(|c| helpers.contains(c.as_str()))
                .take(CALLS)
                .collect();
            if !calls.is_empty() {
                state["calls"] = json!(calls);
            }
            Member {
                name: case.name.clone(),
                line: case.line,
                end_line: case.end_line,
                state,
            }
        })
        .chain(
            support
                .iter()
                .map(|unit| unit_member(unit, &helpers, &BTreeSet::new())),
        )
        .collect();
    let sets = groups::test_groups(cases, &support);
    plan_outline(file, true, listed, sets, out, requests);
}

/// The outline unit and its request, which asks the look-here question;
/// `sets` are groups of positions in `listed`, sent as evidence.
fn plan_outline(
    file: &FileContext<'_>,
    tests: bool,
    listed: Vec<Member>,
    sets: Vec<Vec<usize>>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let groups: Vec<Value> = sets
        .iter()
        .enumerate()
        .map(|(i, set)| json!({"id": format!("G{}", i + 1), "members": set.iter().map(|&p| &listed[p].name).collect::<Vec<_>>()}))
        .collect();
    let mut questions = Questions::default();
    questions.ask(
        LOOK.into(),
        questions::outline_look(tests),
        ID,
        FILE_ORGANIZATION,
        LOOK,
        Pass::First,
    );
    let mut state = json!({
        "file": file.plain_state(),
        "members": listed.iter().map(|m| &m.state).collect::<Vec<_>>(),
        "groups": groups,
    });
    state["file"]["lines"] = json!(file.source.lines().count());
    let (request, asked) = file.request("outline", state, questions);
    let fits = file.budget.fits_structured(&request);
    // A short file is read in one pass; splitting it is not a maintainability gain.
    let floor = if crate::analysis::bend::file(file.path) {
        MIN_BEND_FILE_LINES
    } else {
        MIN_FILE_LINES
    };
    let small = member_code_lines(file.source, &listed) < floor;
    let first = listed.iter().map(|m| m.line).min().unwrap_or(1);
    let last = listed.iter().map(|m| m.end_line).max().unwrap_or(first);
    let names: Vec<&str> = listed.iter().map(|m| m.name.as_str()).collect();
    out.units.push(UnitPlan {
        rule: FILE_ORGANIZATION,
        id: ID.into(),
        name: file.path.display().to_string(),
        presence: if small {
            Presence::TooSmall
        } else {
            Presence::judged_if(fits)
        },
        locations: vec![file.location(first, last, None)],
        quote: None,
        lines: file.source.lines().count(),
        identity: identity(&names),
        detail: Detail::Outline { tests },
        recheck: None,
    });
    if fits && !small {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
    }
}

const ID: &str = "outline";

/// Each member's name, kind, size, signature, doc line, calls to other
/// members, and a few files that import this one and call it.
fn member_state(
    file: &FileContext<'_>,
    units: &[Unit],
    members: &[usize],
    callers: &BTreeMap<String, BTreeSet<PathBuf>>,
) -> Vec<Member> {
    // Member names and the types that own member methods.
    let names: BTreeSet<&str> = members
        .iter()
        .flat_map(|&m| [units[m].short_name.as_str(), units[m].owner.as_str()])
        .filter(|name| !name.is_empty())
        .collect();
    members
        .iter()
        .map(|&m| {
            let unit = &units[m];
            let used_by: BTreeSet<&PathBuf> = callers
                .get(&unit.short_name)
                .into_iter()
                .flatten()
                .filter(|path| path.as_path() != file.path)
                .take(USED_BY)
                .collect();
            unit_member(unit, &names, &used_by)
        })
        .collect()
}

fn unit_member(unit: &Unit, names: &BTreeSet<&str>, used_by: &BTreeSet<&PathBuf>) -> Member {
    let calls: Vec<&String> = unit
        .calls
        .iter()
        .filter(|call| names.contains(call.as_str()) && **call != unit.short_name)
        .take(CALLS)
        .collect();
    let mut state = json!({
        "name": unit.name,
        "kind": match unit.kind {
            Kind::Function => "function",
            Kind::Method => "method",
            Kind::Type => "type",
            Kind::Law => "law",
        },
        "lines": unit.lines(),
        "signature": unit.signature,
    });
    if !unit.doc.is_empty() {
        state["doc"] = json!(unit.doc);
    }
    if !calls.is_empty() {
        state["calls"] = json!(calls);
    }
    if !used_by.is_empty() {
        state["used_by"] = json!(used_by);
    }
    Member {
        name: unit.name.clone(),
        line: unit.line,
        end_line: unit.end_line,
        state,
    }
}

/// Non-blank lines inside members, so test code outside them does not count.
fn member_code_lines(source: &str, listed: &[Member]) -> usize {
    let covered: BTreeSet<usize> = listed.iter().flat_map(|m| m.line..=m.end_line).collect();
    source
        .lines()
        .enumerate()
        .filter(|(i, line)| covered.contains(&(i + 1)) && !line.trim().is_empty())
        .count()
}
