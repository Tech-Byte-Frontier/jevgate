//! Agent instruction files: each heading section is a unit, packed by file,
//! beside what the repository's own files show. Per section, a Score on
//! whether an agent could learn it from those files and Nouls on generic
//! advice, past work and rules linters check; for text loaded in every
//! session, a Choice on whether it applies to one directory only. A section
//! whose signals stay undecided is asked, alone, what kind of section it is.
use super::{
    Detail, FileContext, FilePlan, PACK_ITEMS, Planned, Presence, Questions, UnitPlan, compact,
    identity, pack, questions, unique_ids,
};
use crate::{
    catalog::AGENT_CONTEXT,
    docs::{
        Repository,
        load::{Load, Reader},
        markdown,
    },
    schema::Pass,
};
use serde_json::{Value, json};

/// A section of the text before the first heading is named after the file.
pub(super) const PREAMBLE: &str = "text before the first heading";

pub(super) fn plan(
    file: &FileContext<'_>,
    repository: &Repository,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    out.rules.insert(AGENT_CONTEXT, 0);
    let readers = repository
        .readers
        .get(file.path)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let every_session = readers.iter().any(|r| r.load == Load::Always);
    let loaded = loaded(readers);
    let (sections, names) = units(file.source);
    let ids = unique_ids("section", names.iter().map(String::as_str));
    let mut items = Vec::new();
    for ((section, name), id) in sections.iter().zip(&names).zip(ids) {
        out.units.push(UnitPlan {
            rule: AGENT_CONTEXT,
            id: id.clone(),
            name: name.clone(),
            presence: Presence::Judged,
            locations: vec![file.location(section.start_line, section.end_line, Some(name))],
            quote: None,
            lines: section.end_line + 1 - section.start_line,
            identity: identity(&[&section.heading, &compact(&section.text)]),
            detail: Detail::Section {
                tokens: markdown::tokens(&section.text),
                loaded: loaded.clone(),
            },
            recheck: None,
        });
        let state = json!({"heading": section.heading, "text": section.text});
        items.push((out.units.len() - 1, id, state));
    }
    let evidence = Evidence {
        project: json!({
            "manifests": nearest_manifests(&repository.project.manifests, file.path),
            "directories": repository.project.directories,
        }),
        linters: &repository.project.linters,
        directories: if every_session && repository.project.directories.len() >= 2 {
            &repository.project.directories
        } else {
            &[]
        },
    };
    for (index, id, state) in &items {
        let item = (*index, id.clone(), state.clone());
        let (request, asked) = kind_request(file, &evidence, &item);
        if file.budget.fits(&request) {
            out.units[*index].recheck = Some((request, asked));
        }
    }
    for group in pack(items, PACK_ITEMS, |(_, _, state)| state) {
        send_or_split(file, &evidence, group, out, requests);
    }
}

/// The kind of one section, asked apart from its first request so it never
/// moves the first answers.
fn kind_request(
    file: &FileContext<'_>,
    evidence: &Evidence<'_>,
    item: &(usize, String, Value),
) -> (Value, super::Asked) {
    let mut questions = Questions::default();
    questions.ask(
        "kind".into(),
        questions::instructions_kind("sections[0]"),
        &item.1,
        AGENT_CONTEXT,
        "kind",
        Pass::Recheck,
    );
    let state = json!({
        "file": {"path": file.path},
        "project": evidence.project,
        "sections": [item.2],
    });
    file.request("recheck", state, questions)
}

/// Sections longer than this many bytes (about 400 tokens) are judged by
/// their top-level blocks, so a finding points at the paragraph or list item
/// to change.
const LONG_SECTION_BYTES: usize = 1_600;

/// The units of a file, heading sections or the blocks of long ones, and
/// their names.
fn units(source: &str) -> (Vec<markdown::Section>, Vec<String>) {
    let mut units = Vec::new();
    let mut names = Vec::new();
    for section in markdown::parse(source).sections {
        // Imports alone point at another file; that file's sections are judged.
        if section
            .text
            .split_whitespace()
            .all(|word| word.starts_with('@'))
        {
            continue;
        }
        let heading = if section.heading.is_empty() {
            PREAMBLE.to_string()
        } else {
            section.heading.clone()
        };
        let blocks = markdown::blocks(source, &section);
        if section.text.len() > LONG_SECTION_BYTES && blocks.len() > 1 {
            for block in blocks {
                names.push(format!("{heading}, line {}", block.start_line));
                units.push(block);
            }
        } else {
            names.push(heading);
            units.push(section);
        }
    }
    (units, names)
}

/// Manifests sent with one file's sections.
const MANIFESTS: usize = 12;

/// The manifests nearest the file: those in its own directory and the ones
/// above it first, then the shallowest others.
fn nearest_manifests(manifests: &[Value], path: &std::path::Path) -> Vec<Value> {
    let dir = path.parent().unwrap_or(std::path::Path::new(""));
    let mut ranked: Vec<(bool, usize, &Value)> = manifests
        .iter()
        .map(|m| {
            let at = std::path::Path::new(m["path"].as_str().unwrap_or(""))
                .parent()
                .unwrap_or(std::path::Path::new(""))
                .to_path_buf();
            (!dir.starts_with(&at), at.components().count(), m)
        })
        .collect();
    ranked.sort_by_key(|(outside, depth, _)| (*outside, *depth));
    ranked
        .into_iter()
        .take(MANIFESTS)
        .map(|(_, _, m)| m.clone())
        .collect()
}

/// What every request about one file carries beside its sections.
struct Evidence<'a> {
    project: Value,
    linters: &'a [String],
    /// Scope options, only for text loaded in every session.
    directories: &'a [String],
}

/// A pack that is too large is sent one section at a time.
fn send_or_split(
    file: &FileContext<'_>,
    evidence: &Evidence<'_>,
    group: Vec<(usize, String, Value)>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (request, asked) = sections_request(file, evidence, &group);
    if file.budget.fits(&request) {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
        return;
    }
    for item in group {
        let (request, asked) = sections_request(file, evidence, std::slice::from_ref(&item));
        if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        } else {
            out.units[item.0].presence = Presence::NeedsContext;
        }
    }
}

fn sections_request(
    file: &FileContext<'_>,
    evidence: &Evidence<'_>,
    items: &[(usize, String, Value)],
) -> (Value, super::Asked) {
    let mut questions = Questions::default();
    for (index, (_, id, _)) in items.iter().enumerate() {
        let section = format!("sections[{index}]");
        let mut asked = vec![
            ("inferable", questions::instructions_inferable(&section)),
            ("describes", questions::instructions_describes(&section)),
            ("generic", questions::instructions_generic(&section)),
            ("history", questions::instructions_history(&section)),
        ];
        if evidence.project["manifests"]
            .as_array()
            .is_some_and(|m| !m.is_empty())
        {
            asked.push(("commands", questions::instructions_commands(&section)));
        }
        if !evidence.linters.is_empty() {
            asked.push(("enforced", questions::instructions_enforced(&section)));
        }
        if !evidence.directories.is_empty() {
            asked.push((
                "scope",
                questions::instructions_scope(&section, evidence.directories),
            ));
        }
        for (question, body) in asked {
            questions.ask(
                format!("s{index}_{question}"),
                body,
                id,
                AGENT_CONTEXT,
                question,
                Pass::First,
            );
        }
    }
    let mut state = json!({
        "file": {"path": file.path},
        "project": evidence.project,
        "sections": items.iter().map(|(_, _, state)| state.clone()).collect::<Vec<_>>(),
    });
    if !evidence.linters.is_empty() {
        state["linters"] = json!(evidence.linters);
    }
    file.request("instructions", state, questions)
}

/// Which harnesses load the file and when, for the finding's message.
fn loaded(readers: &[Reader]) -> String {
    let mut by_load: Vec<(&Load, Vec<&str>)> = Vec::new();
    for reader in readers.iter().filter(|r| r.load != Load::Manual) {
        match by_load.iter_mut().find(|(load, _)| **load == reader.load) {
            Some((_, harnesses)) => harnesses.push(&reader.harness),
            None => by_load.push((&reader.load, vec![&reader.harness])),
        }
    }
    by_load.sort_by_key(|(load, _)| **load != Load::Always);
    by_load
        .iter()
        .map(|(load, harnesses)| {
            let verb = if harnesses.len() == 1 {
                "loads"
            } else {
                "load"
            };
            format!("{} {verb} it {}", list(harnesses), load.phrase())
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).into(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
