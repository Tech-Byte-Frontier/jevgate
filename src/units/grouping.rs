//! Hardcoded-value findings that name the same special-cased identity in
//! different files become one finding: the strongest lists the other sites,
//! which become notes pointing at it. The shared value only groups findings
//! Jev already raised and whose value Jev named; it never raises one.
use super::compose::{counted_status, file_status};
use crate::{
    catalog,
    schema::{FileResult, Location, Strength},
};
use std::collections::BTreeSet;

/// A value must be this long to link findings, so short numbers and empty
/// strings never chain unrelated code together.
const LINKING_VALUE_CHARS: usize = 4;
/// Other sites named in a grouped finding's message.
const NAMED_SITES: usize = 3;

/// A finding by file index and finding index.
type At = (usize, usize);

pub fn group_repeated_values(files: &mut [FileResult]) {
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
        let (f, i) = primary;
        let at = format!("{}:{}", files[f].path.display(), files[f].findings[i].line);
        let mut sites = Vec::new();
        let mut locations = Vec::new();
        for (member, value) in members {
            grouped.insert(member);
            changed.insert(member.0);
            let (site, location) = demote(files, member, &value, &at);
            sites.push(site);
            locations.extend(location);
        }
        describe(&mut files[f].findings[i], &sites, locations);
    }
    for g in changed {
        let file = &mut files[g];
        file.findings
            .sort_by(|a, b| b.strength.cmp(&a.strength).then(b.rank.total_cmp(&a.rank)));
        file.status = file_status(&file.dimensions, &file.findings);
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

/// Turn a repeat into a note pointing at the primary site, keeping its
/// file's counts; returns its site and location for the primary.
fn demote(
    files: &mut [FileResult],
    (g, j): At,
    value: &str,
    at: &str,
) -> (String, Option<Location>) {
    let file = &mut files[g];
    let member = &mut file.findings[j];
    let site = format!("{}:{}", file.path.display(), member.line);
    let location = member.locations.first().cloned();
    let was = std::mem::replace(&mut member.strength, Strength::Note);
    member.message = format!("{} Same value {value} as {at}.", member.message);
    if let Some(dimension) = file.dimensions.get_mut(catalog::HARDCODED_VALUES) {
        let count = &mut dimension.units;
        *match was {
            Strength::Review => &mut count.review,
            _ => &mut count.consider,
        } -= 1;
        count.note += 1;
        dimension.status = counted_status(count);
    }
    (site, location)
}

/// Name the other sites in the primary finding and add their locations.
fn describe(primary: &mut crate::schema::Finding, sites: &[String], locations: Vec<Location>) {
    let shown = sites.iter().take(NAMED_SITES).cloned().collect::<Vec<_>>();
    let more = match sites.len().saturating_sub(NAMED_SITES) {
        0 => String::new(),
        more => format!(" and {more} more"),
    };
    primary.message = format!(
        "{} The same value is also flagged at {}{more}.",
        primary.message,
        shown.join(", ")
    );
    primary.locations.extend(locations);
}
