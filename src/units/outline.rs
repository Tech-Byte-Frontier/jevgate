//! File organization: an outline of member signatures and groups, without bodies.
use super::{
    Asked, Detail, FileContext, FilePlan, GroupInfo, Planned, Presence, UnitPlan, identity,
    questions,
};
use crate::{
    analysis::{
        groups,
        units::{FileUnits, Kind},
    },
    catalog::FILE_ORGANIZATION,
    requests::TokenBudget,
    schema::Pass,
};
use serde_json::{Map, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

const CALLS: usize = 12;
const USED_BY: usize = 3;

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
    let names: BTreeSet<&str> = members
        .iter()
        .map(|&m| units[m].short_name.as_str())
        .collect();
    let member_state: Vec<_> = members
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
        .collect();
    let group_state: Vec<_> = groups
        .iter()
        .map(|g| json!({"id": g.id, "members": g.members.iter().map(|&m| &units[m].name).collect::<Vec<_>>()}))
        .collect();
    let id = "outline".to_string();
    let mut questions = Map::new();
    let mut asked = Asked::default();
    asked.ask(
        &mut questions,
        "purpose".into(),
        questions::outline_purpose(),
        &id,
        FILE_ORGANIZATION,
        "purpose",
        Pass::First,
    );
    if groups.len() > 1 {
        let ids: Vec<String> = groups.iter().map(|g| g.id.clone()).collect();
        asked.ask(
            &mut questions,
            "module".into(),
            questions::outline_module(&ids),
            &id,
            FILE_ORGANIZATION,
            "module",
            Pass::First,
        );
        for a in 0..groups.len() {
            for b in 0..groups.len() {
                if a != b {
                    asked.ask(
                        &mut questions,
                        format!("independent_{a}_{b}"),
                        questions::outline_independent(a, b),
                        &format!("{id}:{}:{}", groups[a].id, groups[b].id),
                        FILE_ORGANIZATION,
                        "independent",
                        Pass::First,
                    );
                }
            }
        }
    }
    let state = json!({
        "file": file.file_state(),
        "members": member_state,
        "groups": group_state,
    });
    let request = file.request("outline", state, questions);
    let fits = budget.fits(&request);
    let first = members.iter().map(|&m| units[m].line).min().unwrap_or(1);
    let last = members
        .iter()
        .map(|&m| units[m].end_line)
        .max()
        .unwrap_or(first);
    let member_names: Vec<&str> = members.iter().map(|&m| units[m].name.as_str()).collect();
    out.units.push(UnitPlan {
        rule: FILE_ORGANIZATION,
        id,
        name: file.path.display().to_string(),
        presence: if fits {
            Presence::Judged
        } else {
            Presence::NeedsContext
        },
        locations: vec![file.location(first, last, None)],
        quote: None,
        lines: file.source.lines().count(),
        identity: identity(&member_names),
        detail: Detail::Outline {
            groups: groups
                .iter()
                .map(|g| GroupInfo {
                    id: g.id.clone(),
                    names: g.members.iter().map(|&m| units[m].name.clone()).collect(),
                    locations: g
                        .members
                        .iter()
                        .map(|&m| {
                            file.location(units[m].line, units[m].end_line, Some(&units[m].name))
                        })
                        .collect(),
                })
                .collect(),
        },
        recheck: None,
    });
    if fits {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
    }
}
