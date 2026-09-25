//! File organization: an outline of member signatures, sizes and groups,
//! without bodies. A test file's members are its test cases and the support
//! code they share.
use super::{
    Asked, Detail, FileContext, FilePlan, GroupInfo, Planned, Presence, Questions, UnitPlan,
    identity, questions,
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
    let source = application_source(file.source, units, members);
    plan_outline(file, false, listed, sets, source, out, requests);
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
    plan_outline(
        file,
        true,
        listed,
        sets,
        file.source.to_string(),
        out,
        requests,
    );
}

/// The outline unit and its request; `sets` are groups of positions in `listed`.
fn plan_outline(
    file: &FileContext<'_>,
    tests: bool,
    listed: Vec<Member>,
    sets: Vec<Vec<usize>>,
    source: String,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let ids: Vec<String> = (1..=sets.len()).map(|i| format!("G{i}")).collect();
    let outline = Outline {
        tests,
        lines: file.source.lines().count(),
        members: listed.iter().map(|m| m.state.clone()).collect(),
        groups: ids
            .iter()
            .zip(&sets)
            .map(|(id, set)| json!({"id": id, "members": set.iter().map(|&p| &listed[p].name).collect::<Vec<_>>()}))
            .collect(),
        ids: ids.clone(),
    };
    let (request, asked) = outline.request(file, Ask::First);
    let fits = file.budget.fits(&request);
    // A short file is read in one pass; splitting it is not a maintainability gain.
    let small = member_code_lines(file.source, &listed) < MIN_FILE_LINES;
    let judged = fits && !small;
    let first = listed.iter().map(|m| m.line).min().unwrap_or(1);
    let last = listed.iter().map(|m| m.end_line).max().unwrap_or(first);
    let names: Vec<&str> = listed.iter().map(|m| m.name.as_str()).collect();
    out.units.push(UnitPlan {
        rule: FILE_ORGANIZATION,
        id: ID.into(),
        name: file.path.display().to_string(),
        presence: if small {
            Presence::TooSmall
        } else if fits {
            Presence::Judged
        } else {
            Presence::NeedsContext
        },
        locations: vec![file.location(first, last, None)],
        quote: None,
        lines: file.source.lines().count(),
        identity: identity(&names),
        detail: Detail::Outline {
            tests,
            kind: judged
                .then(|| outline.request(file, Ask::Kind(source.clone())))
                .filter(|(request, _)| file.budget.fits(request)),
            groups: ids
                .into_iter()
                .zip(&sets)
                .map(|(id, set)| GroupInfo {
                    id,
                    names: set.iter().map(|&p| listed[p].name.clone()).collect(),
                    locations: set
                        .iter()
                        .map(|&p| {
                            let m = &listed[p];
                            file.location(m.line, m.end_line, Some(&m.name))
                        })
                        .collect(),
                })
                .collect(),
        },
        recheck: judged
            .then(|| outline.request(file, Ask::Recheck(source.clone())))
            .filter(|(request, _)| file.budget.fits(request)),
    });
    if judged {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
    }
}

const ID: &str = "outline";

/// The uploaded outline: members, their groups, and the group IDs as options.
struct Outline {
    tests: bool,
    lines: usize,
    members: Vec<Value>,
    groups: Vec<Value>,
    ids: Vec<String>,
}

/// The requests about one outline, in the order they may be asked.
enum Ask {
    /// Signatures only.
    First,
    /// The split again with the file's application source.
    Recheck(String),
    /// What kind of file it is, asked only when the recheck stays undecided.
    Kind(String),
}

impl Outline {
    fn request(&self, file: &FileContext<'_>, ask: Ask) -> (Value, Asked) {
        let mut questions = Questions::default();
        let (pass, stage, source) = match ask {
            Ask::First => (Pass::First, "outline", None),
            Ask::Recheck(source) => (Pass::Recheck, "recheck", Some(source)),
            Ask::Kind(source) => (Pass::Trace, "trace", Some(source)),
        };
        if pass == Pass::Trace {
            // A separate request, so the kind never moves the split answers.
            questions.ask(
                "kind".into(),
                questions::outline_kind(self.tests),
                ID,
                FILE_ORGANIZATION,
                "kind",
                pass,
            );
        } else {
            questions.ask(
                "split".into(),
                questions::outline_split(self.tests, source.is_some()),
                ID,
                FILE_ORGANIZATION,
                "split",
                pass,
            );
            if self.ids.len() > 1 {
                // Speculative location: consumed only when the split Score raises a finding.
                questions.ask(
                    "module".into(),
                    questions::outline_module(self.tests, &self.ids),
                    ID,
                    FILE_ORGANIZATION,
                    "module",
                    pass,
                );
            }
        }
        let mut state = json!({
            "file": file.plain_state(),
            "members": self.members,
            "groups": self.groups,
        });
        state["file"]["lines"] = json!(self.lines);
        if let Some(source) = source {
            state["file"]["source"] = json!(source);
        }
        file.request(stage, state, questions)
    }
}

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

/// The file without the lines of units that are not members, such as tests.
fn application_source(source: &str, units: &[Unit], members: &[usize]) -> String {
    let excluded: Vec<(usize, usize)> = (0..units.len())
        .filter(|i| !members.contains(i))
        .map(|i| (units[i].line, units[i].end_line))
        .collect();
    source
        .lines()
        .enumerate()
        .filter(|(i, _)| !excluded.iter().any(|&(a, b)| (a..=b).contains(&(i + 1))))
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}
