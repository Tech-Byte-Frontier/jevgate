//! Repeated findings become one. Finished plans in one directory: the first
//! lists the others, and the
//! finding is identified by the directory, so a baseline survives plans
//! being added or removed. A section repeated in several documents: the
//! repetition findings that share a section are one finding at the section
//! most of them name, identified by it, and the others point at it.
use super::compose::{counted_status, reorder};
use crate::{
    catalog,
    schema::{FileResult, Finding, Location, Strength},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// The category of a finished-plan finding, which groups by directory.
pub const FINISHED_PLAN: &str = "finished plan";

/// Other sites named in a grouped finding's message.
const NAMED_SITES: usize = 3;

/// A finding by file index and finding index.
type At = (usize, usize);

/// Group every kind of repeated finding, then restore each changed file's order and status.
pub fn group_repeats(files: &mut [FileResult]) {
    let mut changed = group_finished_plans(files);
    changed.extend(group_repeated_sections(files));
    for g in changed {
        reorder(&mut files[g]);
    }
}

/// Finished plans that share a directory: the first by path keeps the
/// finding and names the others, which become notes pointing at it. Returns
/// the files whose findings changed.
fn group_finished_plans(files: &mut [FileResult]) -> BTreeSet<usize> {
    let rule = catalog::id(catalog::DOC_STALENESS);
    let mut directories = BTreeMap::<PathBuf, Vec<At>>::new();
    for (f, file) in files.iter().enumerate() {
        for (i, finding) in file.findings.iter().enumerate() {
            if finding.rule == rule
                && finding.strength != Strength::Note
                && finding.category.as_deref() == Some(FINISHED_PLAN)
            {
                let directory = file.path.parent().unwrap_or(Path::new("")).to_path_buf();
                directories.entry(directory).or_default().push((f, i));
            }
        }
    }
    let mut changed = BTreeSet::new();
    for (directory, mut plans) in directories {
        if plans.len() < 2 {
            continue;
        }
        plans.sort_by(|a, b| files[a.0].path.cmp(&files[b.0].path));
        let (f, i) = plans[0];
        let at = site(files, plans[0]);
        let pointer = format!(
            " Grouped with the other finished plans in `{}` at {at}.",
            directory.display()
        );
        let pointed: Vec<(At, String)> = plans[1..].iter().map(|m| (*m, pointer.clone())).collect();
        let names: Vec<String> = pointed
            .iter()
            .map(|((g, _), _)| format!("`{}`", files[*g].path.display()))
            .collect();
        changed.extend(plans.iter().map(|(g, _)| *g));
        let locations = lower_members(files, &pointed, catalog::DOC_STALENESS);
        let primary = &mut files[f].findings[i];
        primary.message = format!(
            "{} The other {} plans in `{}` are finished too: {}.",
            primary.message,
            names.len(),
            directory.display(),
            listed(&names)
        );
        primary.action =
            "Delete the finished plans, or move them out of the living documentation".into();
        primary.locations.extend(locations);
        primary.fingerprint = directory_fingerprint(primary, &directory);
        primary.identity = Default::default();
    }
    changed
}

/// The category of a grouped repetition finding.
pub const REPEATED_SECTION: &str = "repeated section";

/// Repetition findings linked by the sections they name: the finding at the
/// section most of them name keeps a finding listing every other document,
/// and the others become notes pointing at it. Disagreements are left apart,
/// since each names its own fix. Returns the files whose findings changed.
fn group_repeated_sections(files: &mut [FileResult]) -> BTreeSet<usize> {
    let repeats = repetitions(files);
    let mut changed = BTreeSet::new();
    for members in linked_by_sections(&repeats) {
        let members: Vec<&(At, [Section; 2])> = members.into_iter().map(|n| &repeats[n]).collect();
        let Some(head) = head_section(&members) else {
            continue;
        };
        let Some((primary, shown)) = members
            .iter()
            .filter(|(_, sections)| sections.contains(&head))
            .min_by_key(|((f, i), _)| (&files[*f].path, files[*f].findings[*i].line))
            .map(|(at, sections)| (*at, sections.clone()))
        else {
            continue;
        };
        let names = other_sections(files, &members, &shown);
        let pointer = format!(
            " Grouped with the other copies of this section at {}.",
            site(files, primary)
        );
        let pointed: Vec<(At, String)> = members
            .iter()
            .map(|(at, _)| *at)
            .filter(|at| *at != primary)
            .map(|at| (at, pointer.clone()))
            .collect();
        changed.extend(members.iter().map(|((g, _), _)| *g));
        // Every section of the group, not only the side each member owns.
        let locations: Vec<Location> = pointed
            .iter()
            .flat_map(|((g, j), _)| files[*g].findings[*j].locations[..2].to_vec())
            .collect();
        lower_members(files, &pointed, catalog::DOC_DUPLICATION);
        head_finding(
            &mut files[primary.0].findings[primary.1],
            &head,
            &names,
            locations,
        );
    }
    changed
}

/// Each repetition finding with the two sections it names.
fn repetitions(files: &[FileResult]) -> Vec<(At, [Section; 2])> {
    let rule = catalog::id(catalog::DOC_DUPLICATION);
    files
        .iter()
        .enumerate()
        .flat_map(|(f, file)| {
            file.findings.iter().enumerate().filter_map(move |(i, x)| {
                let named = (x.rule == rule
                    && x.strength != Strength::Note
                    && x.category.is_none()
                    && x.locations.len() >= 2)
                    .then(|| [section(&x.locations[0]), section(&x.locations[1])])?;
                Some(((f, i), named))
            })
        })
        .collect()
}

/// The head of a group: the section most findings name, first by path and line.
fn head_section(members: &[&(At, [Section; 2])]) -> Option<Section> {
    let mut named = BTreeMap::<&Section, usize>::new();
    for (_, sections) in members {
        for s in sections {
            *named.entry(s).or_default() += 1;
        }
    }
    let most = named.values().copied().max().unwrap_or(0);
    named
        .iter()
        .find(|(_, n)| **n == most)
        .map(|(s, _)| (*s).clone())
}

/// The group's sections the head finding does not show, by document; a
/// document already shown, or named twice, is named with the section's
/// heading.
fn other_sections(
    files: &[FileResult],
    members: &[&(At, [Section; 2])],
    shown: &[Section; 2],
) -> Vec<String> {
    let others: BTreeSet<&Section> = members
        .iter()
        .flat_map(|(_, sections)| sections.iter())
        .filter(|s| !shown.contains(s))
        .collect();
    let heading_of = |s: &Section| {
        members
            .iter()
            .flat_map(|((f, i), _)| files[*f].findings[*i].locations.iter())
            .find(|l| section(l) == *s)
            .and_then(|l| l.symbol.clone())
            .unwrap_or_default()
    };
    others
        .iter()
        .map(|s| {
            let named_elsewhere = shown.iter().any(|x| x.0 == s.0)
                || others.iter().filter(|o| o.0 == s.0).count() > 1;
            if named_elsewhere {
                format!("section `{}` of `{}`", heading_of(s), s.0.display())
            } else {
                format!("`{}`", s.0.display())
            }
        })
        .collect()
}

/// The finding a group keeps: it names the other sections, gains their
/// locations and is identified by the head section.
fn head_finding(finding: &mut Finding, head: &Section, names: &[String], locations: Vec<Location>) {
    if !names.is_empty() {
        let one = names.len() == 1;
        finding.message = format!(
            "{} The same text recurs in {} more section{}: {}.",
            finding.message,
            names.len(),
            if one { "" } else { "s" },
            listed(names)
        );
    }
    finding.action =
        "Keep one copy, such as a shared partial, and include or link it from the other documents"
            .into();
    for location in locations {
        if !finding
            .locations
            .iter()
            .any(|l| section(l) == section(&location))
        {
            finding.locations.push(location);
        }
    }
    let heading = finding
        .locations
        .iter()
        .find(|l| section(l) == *head)
        .and_then(|l| l.symbol.clone())
        .unwrap_or_default();
    finding.category = Some(REPEATED_SECTION.into());
    finding.fingerprint = section_fingerprint(finding, &head.0, &heading);
    finding.identity = Default::default();
}

/// A section a finding names: its path and first line.
type Section = (PathBuf, usize);

fn section(location: &Location) -> Section {
    (location.path.clone(), location.start_line)
}

/// Groups of two or more findings, as indexes, linked through the sections
/// they name.
fn linked_by_sections(repeats: &[(At, [Section; 2])]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..repeats.len()).collect();
    fn root(parent: &mut [usize], mut n: usize) -> usize {
        while parent[n] != n {
            parent[n] = parent[parent[n]];
            n = parent[n];
        }
        n
    }
    let mut first = BTreeMap::<&Section, usize>::new();
    for (n, (_, sections)) in repeats.iter().enumerate() {
        for s in sections {
            if let Some(&m) = first.get(s) {
                let (a, b) = (root(&mut parent, n), root(&mut parent, m));
                parent[a.max(b)] = a.min(b);
            } else {
                first.insert(s, n);
            }
        }
    }
    let mut groups = BTreeMap::<usize, Vec<usize>>::new();
    for n in 0..repeats.len() {
        let g = root(&mut parent, n);
        groups.entry(g).or_default().push(n);
    }
    groups.into_values().filter(|g| g.len() >= 2).collect()
}

/// A grouped repetition's identity: its rule, the document and heading of
/// the section most copies repeat.
fn section_fingerprint(finding: &Finding, path: &Path, heading: &str) -> String {
    let separator = crate::schema::HASH_SEPARATOR;
    crate::schema::hash(
        format!(
            "{}{separator}{REPEATED_SECTION}{separator}{}{separator}{heading}",
            finding.rule,
            path.display()
        )
        .as_bytes(),
    )
}

/// A grouped finding's identity: its rule, category and directory, so it
/// stays the same as members come and go.
fn directory_fingerprint(finding: &Finding, directory: &Path) -> String {
    let separator = crate::schema::HASH_SEPARATOR;
    crate::schema::hash(
        format!(
            "{}{separator}{}{separator}{}",
            finding.rule,
            finding.category.as_deref().unwrap_or_default(),
            directory.display()
        )
        .as_bytes(),
    )
}

/// Make finding `j` of `file` a note, which one level does not report, and
/// move it from the reviews of rule `key`, where composition counted it, to
/// its notes.
fn lower_to_note(file: &mut FileResult, j: usize, key: &str) {
    file.findings[j].strength = Strength::Note;
    if let Some(dimension) = file.dimensions.get_mut(key) {
        let count = &mut dimension.units;
        count.review = count.review.saturating_sub(1);
        count.note += 1;
        dimension.status = counted_status(count);
    }
}

/// A finding's `path:line`.
fn site(files: &[FileResult], (f, i): At) -> String {
    format!("{}:{}", files[f].path.display(), files[f].findings[i].line)
}

/// Turn each member into a note whose message ends with its pointer to the
/// primary, keeping its file's counts for rule `key`; returns their first
/// locations for the primary.
fn lower_members(files: &mut [FileResult], members: &[(At, String)], key: &str) -> Vec<Location> {
    let mut locations = Vec::new();
    for ((g, j), pointer) in members {
        let file = &mut files[*g];
        let member = &mut file.findings[*j];
        locations.extend(member.locations.first().cloned());
        member.message.push_str(pointer);
        lower_to_note(file, *j, key);
    }
    locations
}

/// The first few of `names`, then how many more there are.
fn listed(names: &[String]) -> String {
    let shown = names.iter().take(NAMED_SITES).cloned().collect::<Vec<_>>();
    match names.len().saturating_sub(NAMED_SITES) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}
