//! File organization: an outline of member signatures and groups, without bodies.
use super::{
    Asked, Detail, FileContext, FilePlan, GroupInfo, Planned, Presence, UnitPlan, identity,
    questions,
};
use crate::{
    analysis::{
        groups,
        units::{FileUnits, Kind, Unit},
    },
    catalog::FILE_ORGANIZATION,
    schema::Pass,
    token_budget::TokenBudget,
};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

const CALLS: usize = 12;
const USED_BY: usize = 3;
/// Files with fewer non-blank lines are too small to split.
pub const MIN_FILE_LINES: usize = 100;

pub(super) fn plan(
    file: &FileContext<'_>,
    parsed: &FileUnits,
    members: &[usize],
    callers: &BTreeMap<String, BTreeSet<PathBuf>>,
    budget: &TokenBudget,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let units = &parsed.units;
    let groups = groups::groups(units, members, &parsed.imports);
    let outline = Outline {
        members: member_state(file, units, members, callers),
        groups: groups
            .iter()
            .map(|g| json!({"id": g.id, "members": g.members.iter().map(|&m| &units[m].name).collect::<Vec<_>>()}))
            .collect(),
        ids: groups.iter().map(|g| g.id.clone()).collect(),
    };
    let (request, asked) = outline.request(file, None);
    let fits = budget.fits(&request);
    // A short file is read in one pass; splitting it is not a maintainability gain.
    let small = member_code_lines(file.source, units, members) < MIN_FILE_LINES;
    let judged = fits && !small;
    let first = members.iter().map(|&m| units[m].line).min().unwrap_or(1);
    let last = members
        .iter()
        .map(|&m| units[m].end_line)
        .max()
        .unwrap_or(first);
    let member_names: Vec<&str> = members.iter().map(|&m| units[m].name.as_str()).collect();
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
        identity: identity(&member_names),
        detail: Detail::Outline {
            groups: group_info(file, units, &groups, callers),
        },
        recheck: judged
            .then(|| outline.request(file, Some(application_source(file.source, units, members))))
            .filter(|(request, _)| budget.fits(request)),
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
    members: Vec<Value>,
    groups: Vec<Value>,
    ids: Vec<String>,
}

impl Outline {
    /// The first pass sends signatures only; a recheck adds the file's source.
    fn request(&self, file: &FileContext<'_>, source: Option<String>) -> (Value, Asked) {
        let pass = if source.is_some() {
            Pass::Recheck
        } else {
            Pass::First
        };
        let mut questions = Map::new();
        let mut asked = Asked::default();
        asked.ask(
            &mut questions,
            "split".into(),
            questions::outline_split(source.is_some()),
            ID,
            FILE_ORGANIZATION,
            "split",
            pass,
        );
        if self.ids.len() > 1 {
            // Speculative location: consumed only when the split Score raises a finding.
            asked.ask(
                &mut questions,
                "module".into(),
                questions::outline_module(&self.ids),
                ID,
                FILE_ORGANIZATION,
                "module",
                pass,
            );
        }
        let mut state = json!({
            "file": file.file_state(),
            "members": self.members,
            "groups": self.groups,
        });
        let stage = match source {
            Some(source) => {
                state["file"]["source"] = json!(source);
                "recheck"
            }
            None => "outline",
        };
        (file.request(stage, state, questions), asked)
    }
}

/// Each member's name, kind, signature, doc line, calls to other members,
/// and a few files that import this one and call it.
fn member_state(
    file: &FileContext<'_>,
    units: &[Unit],
    members: &[usize],
    callers: &BTreeMap<String, BTreeSet<PathBuf>>,
) -> Vec<Value> {
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
            let calls: Vec<&String> = unit
                .calls
                .iter()
                .filter(|call| names.contains(call.as_str()) && **call != unit.short_name)
                .take(CALLS)
                .collect();
            let used_by: Vec<&PathBuf> = callers
                .get(&unit.short_name)
                .into_iter()
                .flatten()
                .filter(|path| path.as_path() != file.path)
                .take(USED_BY)
                .collect();
            let mut member = json!({
                "name": unit.name,
                "kind": match unit.kind {
                    Kind::Function => "function",
                    Kind::Method => "method",
                    Kind::Type => "type",
                },
                "signature": unit.signature,
            });
            if !unit.doc.is_empty() {
                member["doc"] = json!(unit.doc);
            }
            if !calls.is_empty() {
                member["calls"] = json!(calls);
            }
            if !used_by.is_empty() {
                member["used_by"] = json!(used_by);
            }
            member
        })
        .collect()
}

/// Non-blank lines inside members, so test code outside them does not count.
fn member_code_lines(source: &str, units: &[Unit], members: &[usize]) -> usize {
    let covered: BTreeSet<usize> = members
        .iter()
        .flat_map(|&m| units[m].line..=units[m].end_line)
        .collect();
    source
        .lines()
        .enumerate()
        .filter(|(i, line)| covered.contains(&(i + 1)) && !line.trim().is_empty())
        .count()
}

fn group_info(
    file: &FileContext<'_>,
    units: &[Unit],
    groups: &[groups::Group],
    callers: &BTreeMap<String, BTreeSet<PathBuf>>,
) -> Vec<GroupInfo> {
    groups
        .iter()
        .map(|g| GroupInfo {
            id: g.id.clone(),
            names: g.members.iter().map(|&m| units[m].name.clone()).collect(),
            users: g
                .members
                .iter()
                .filter_map(|&m| callers.get(&units[m].short_name))
                .flatten()
                .filter(|path| path.as_path() != file.path)
                .cloned()
                .collect(),
            locations: g
                .members
                .iter()
                .map(|&m| file.location(units[m].line, units[m].end_line, Some(&units[m].name)))
                .collect(),
        })
        .collect()
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
