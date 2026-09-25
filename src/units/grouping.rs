//! Repeated findings become one. Hardcoded-value findings that name the same
//! special-cased identity in different files: the strongest lists the other
//! sites, which become notes pointing at it. The shared value only groups
//! findings Jev already raised and whose value Jev named; it never raises one.
//! Finished plans in one directory: the first lists the others, and the
//! finding is identified by the directory, so a baseline survives plans
//! being added or removed. A section repeated in several documents: the
//! repetition findings that share a section are one finding at the section
//! most of them name, identified by it, and the others point at it.
use super::compose::{counted_status, file_status};
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

/// A value must be this long to link findings, so short numbers and empty
/// strings never chain unrelated code together.
const LINKING_VALUE_CHARS: usize = 4;
/// Other sites named in a grouped finding's message.
const NAMED_SITES: usize = 3;

/// A finding by file index and finding index.
type At = (usize, usize);

/// Group every kind of repeated finding, then restore each changed file's order and status.
pub fn group_repeats(files: &mut [FileResult]) {
    let mut changed = group_repeated_values(files);
    changed.extend(group_finished_plans(files));
    changed.extend(group_repeated_sections(files));
    for g in changed {
        let file = &mut files[g];
        file.findings
            .sort_by(|a, b| b.strength.cmp(&a.strength).then(b.rank.total_cmp(&a.rank)));
        file.status = file_status(&file.dimensions, &file.findings);
    }
}

/// Group special-cased identities; returns the files whose findings changed.
fn group_repeated_values(files: &mut [FileResult]) -> BTreeSet<usize> {
    let flagged = flagged(files);
    let mut grouped = BTreeSet::new();
    let mut changed = BTreeSet::new();
    for (n, &primary) in flagged.iter().enumerate() {
        if grouped.contains(&primary) {
            continue;
        }
        let members = members(files, primary, &flagged[n + 1..], &grouped);
        if members.is_empty() {
            continue;
        }
        grouped.insert(primary);
        let at = site(files, primary);
        let pointed: Vec<(At, String)> = members
            .into_iter()
            .map(|(member, value)| (member, format!(" Same value {value} as {at}.")))
            .collect();
        grouped.extend(pointed.iter().map(|(member, _)| *member));
        changed.extend(pointed.iter().map(|((g, _), _)| *g));
        let sites: Vec<String> = pointed.iter().map(|(m, _)| site(files, *m)).collect();
        let locations = lower_members(files, &pointed, catalog::HARDCODED_VALUES);
        let (f, i) = primary;
        describe(&mut files[f].findings[i], &sites, locations);
    }
    changed
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
    let rule = catalog::id(catalog::DOC_DUPLICATION);
    // Each repetition finding with the two sections it names.
    let repeats: Vec<(At, [Section; 2])> = files
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
        .collect();
    let mut changed = BTreeSet::new();
    for members in linked_by_sections(&repeats) {
        let members: Vec<&(At, [Section; 2])> = members.into_iter().map(|n| &repeats[n]).collect();
        // The head: the section most findings name, first by path and line.
        let mut named = BTreeMap::<&Section, usize>::new();
        for (_, sections) in &members {
            for s in sections {
                *named.entry(s).or_default() += 1;
            }
        }
        let most = named.values().copied().max().unwrap_or(0);
        let Some(head) = named
            .iter()
            .find(|(_, n)| **n == most)
            .map(|(s, _)| (*s).clone())
        else {
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
        // The other sections, by document; a document already shown, or
        // named twice, is named with the section's heading.
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
        let names: Vec<String> = others
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
            .collect();
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
        let finding = &mut files[primary.0].findings[primary.1];
        if !names.is_empty() {
            let one = names.len() == 1;
            finding.message = format!(
                "{} The same text recurs in {} more section{}: {}.",
                finding.message,
                names.len(),
                if one { "" } else { "s" },
                listed(&names)
            );
        }
        finding.action = "Keep one copy, such as a shared partial, and include or link it from the other documents".into();
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
            .find(|l| section(l) == head)
            .and_then(|l| l.symbol.clone())
            .unwrap_or_default();
        finding.category = Some(REPEATED_SECTION.into());
        finding.fingerprint = section_fingerprint(finding, &head.0, &heading);
    }
    changed
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

/// Make finding `j` of `file` a note and move it from its level's count to
/// the notes of rule `key`.
fn lower_to_note(file: &mut FileResult, j: usize, key: &str) {
    let was = std::mem::replace(&mut file.findings[j].strength, Strength::Note);
    if let Some(dimension) = file.dimensions.get_mut(key) {
        let count = &mut dimension.units;
        *match was {
            Strength::Review => &mut count.review,
            _ => &mut count.consider,
        } -= 1;
        count.note += 1;
        dimension.status = counted_status(count);
    }
}

/// Reviews and considers that name a value, strongest first.
fn flagged(files: &[FileResult]) -> Vec<At> {
    let rule = catalog::id(catalog::HARDCODED_VALUES);
    let mut flagged: Vec<At> = files
        .iter()
        .enumerate()
        .flat_map(|(f, file)| {
            file.findings.iter().enumerate().filter_map(move |(i, x)| {
                (x.rule == rule && x.strength != Strength::Note && !x.values.is_empty())
                    .then_some((f, i))
            })
        })
        .collect();
    let key = |&(f, i): &At| {
        let x = &files[f].findings[i];
        (x.strength, x.rank)
    };
    flagged.sort_by(|a, b| {
        let (a, b) = (key(a), key(b));
        b.0.cmp(&a.0).then(b.1.total_cmp(&a.1))
    });
    flagged
}

/// Later findings in other files that name one of the primary's values.
fn members(
    files: &[FileResult],
    (f, i): At,
    later: &[At],
    grouped: &BTreeSet<At>,
) -> Vec<(At, String)> {
    let linking: Vec<&String> = files[f].findings[i]
        .values
        .iter()
        .filter(|v| v.chars().count() >= LINKING_VALUE_CHARS)
        .collect();
    later
        .iter()
        .filter(|&&(g, j)| g != f && !grouped.contains(&(g, j)))
        .filter_map(|&(g, j)| {
            let shared = linking
                .iter()
                .find(|v| files[g].findings[j].values.contains(v))?;
            Some(((g, j), (*shared).clone()))
        })
        .collect()
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

/// Name the other sites in the primary finding and add their locations.
fn describe(primary: &mut Finding, sites: &[String], locations: Vec<Location>) {
    primary.message = format!(
        "{} The same value is also flagged at {}.",
        primary.message,
        listed(sites)
    );
    primary.locations.extend(locations);
}

/// The first few of `names`, then how many more there are.
fn listed(names: &[String]) -> String {
    let shown = names.iter().take(NAMED_SITES).cloned().collect::<Vec<_>>();
    match names.len().saturating_sub(NAMED_SITES) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}
