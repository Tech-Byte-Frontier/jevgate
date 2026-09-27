//! Member groups for file organization: a per-file graph of calls, shared
//! owners and shared file-declared types or imports (for a test file, shared
//! suites, subjects and helpers), merged deterministically
//! by average linkage. Label propagation was tried first and let generic
//! shared names pull every member into one group.
use super::{
    test_map::TestCase,
    units::{Kind, Unit},
};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_GROUPS: usize = 6;
/// Clusters merge while the average weight between their members reaches one
/// shared name; a call alone links two members strongly.
const LINK: f64 = 1.0;
/// A call between members links them more strongly than a shared owner.
const CALL_WEIGHT: u32 = 3;
const OWNER_WEIGHT: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// `G1`, `G2`… in order of each group's first member.
    pub id: String,
    /// Indexes into the unit list that was grouped.
    pub members: Vec<usize>,
}

/// Group the selected `members` (indexes into `units`). Deterministic for the same input.
pub fn groups(units: &[Unit], members: &[usize], imports: &BTreeSet<String>) -> Vec<Group> {
    clusters(weights(units, members, imports))
        .into_iter()
        .enumerate()
        .map(|(i, set)| Group {
            id: format!("G{}", i + 1),
            members: set.into_iter().map(|m| members[m]).collect(),
        })
        .collect()
}

/// Candidate parts of a long file, for the part follow-up: `groups` links
/// every method of a class through their owner, so a scraper inside a
/// model or a codec inside a manager never stood apart. Here methods of a
/// type with more than `PART_OWNER_MEMBERS` members link only through calls
/// and shared names, calls to a helper that more than three in ten members
/// call do not link, and members next to each other or sharing a distinctive
/// word of their names link once more. Scored against the parts that
/// labelers named on 23 files, the best matching part rose from 0.45 to
/// 0.72 (F1 over member lines), and from 5 to 9 of the 15 files to split.
pub fn parts(units: &[Unit], members: &[usize], imports: &BTreeSet<String>) -> Vec<Vec<usize>> {
    clusters(part_weights(units, members, imports))
        .into_iter()
        .map(|set| set.into_iter().map(|m| members[m]).collect())
        .collect()
}

/// A type with more members than this holds parts of its own.
const PART_OWNER_MEMBERS: usize = 12;
/// The share of members calling a helper above which calls to it do not link.
const HUB_SHARE: f64 = 0.3;
/// The share of members whose names a word may appear in and still link them.
const DISTINCT_WORD_SHARE: f64 = 0.25;

fn part_weights(units: &[Unit], members: &[usize], imports: &BTreeSet<String>) -> Vec<Vec<u32>> {
    let n = members.len();
    let unit = |i: usize| &units[members[i]];
    let names = linking_names(units, members, imports);
    let mut called = BTreeMap::<&str, usize>::new();
    let mut owners = BTreeMap::<&str, usize>::new();
    let mut words = Vec::with_capacity(n);
    let mut spread = BTreeMap::<String, usize>::new();
    for i in 0..n {
        for call in &unit(i).calls {
            *called.entry(call.as_str()).or_default() += 1;
        }
        if !unit(i).owner.is_empty() {
            *owners.entry(unit(i).owner.as_str()).or_default() += 1;
        }
        let set = name_words(&unit(i).short_name);
        for word in &set {
            *spread.entry(word.clone()).or_default() += 1;
        }
        words.push(set);
    }
    let hub = |name: &str| {
        called
            .get(name)
            .is_some_and(|&c| c as f64 > (n as f64 * HUB_SHARE).max(3.0))
    };
    let calls = |x: &Unit, y: &Unit| {
        !hub(&y.short_name)
            && (x.calls.contains(&y.short_name)
                || !y.owner.is_empty() && x.calls.contains(&y.owner) && !hub(&y.owner))
    };
    let distinct = (n as f64 * DISTINCT_WORD_SHARE).max(2.0);
    let mut weights = vec![vec![0u32; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (unit(i), unit(j));
            let mut weight = names[i].intersection(&names[j]).count().min(2) as u32;
            if calls(a, b) || calls(b, a) {
                weight += CALL_WEIGHT;
            }
            if !a.owner.is_empty()
                && a.owner == b.owner
                && owners[a.owner.as_str()] <= PART_OWNER_MEMBERS
            {
                weight += OWNER_WEIGHT;
            }
            if j == i + 1 {
                weight += 1;
            }
            if words[i]
                .intersection(&words[j])
                .any(|w| spread[w] as f64 <= distinct)
            {
                weight += 1;
            }
            weights[i][j] = weight;
            weights[j][i] = weight;
        }
    }
    weights
}

/// The lowercase words of a name longer than two letters, split at
/// underscores, punctuation and camel-case humps: `fetchedAttributesHtml`
/// and `fetched_attributes_pdf` share `fetched` and `attributes`.
fn name_words(name: &str) -> BTreeSet<String> {
    let mut words = BTreeSet::new();
    let mut word = String::new();
    let mut previous: Option<char> = None;
    for c in name.trim_start_matches(['#', '_']).chars() {
        let hump =
            c.is_uppercase() && previous.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit());
        if !c.is_alphanumeric() || hump {
            if word.chars().count() > 2 {
                words.insert(std::mem::take(&mut word));
            }
            word.clear();
        }
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        }
        previous = Some(c);
    }
    if word.chars().count() > 2 {
        words.insert(word);
    }
    words
}

/// Group a test file's cases and the support code they share: cases link by
/// their innermost suite and by the subjects and helpers they share, and a
/// case that calls a helper links to it. Positions `0..cases.len()` are the
/// cases, then `support` in order. Deterministic for the same input.
pub fn test_groups(cases: &[TestCase], support: &[&Unit]) -> Vec<Vec<usize>> {
    let linking = case_links(cases, support);
    let n = cases.len() + support.len();
    let mut weights = vec![vec![0u32; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let weight = match (cases.get(i), cases.get(j)) {
                (Some(a), Some(b)) => {
                    let suite = !a.suite.is_empty() && a.suite.last() == b.suite.last();
                    u32::from(suite) * OWNER_WEIGHT
                        + linking[i].intersection(&linking[j]).count().min(2) as u32
                }
                (Some(case), None) => {
                    u32::from(case.calls.contains(&support[j - cases.len()].short_name))
                        * CALL_WEIGHT
                }
                _ => link(support[i - cases.len()], support[j - cases.len()]),
            };
            weights[i][j] = weight;
            weights[j][i] = weight;
        }
    }
    clusters(weights)
}

/// Per case, the subjects and helpers it calls that link cases: not a name
/// most cases call, such as a shared setup helper.
fn case_links<'a>(cases: &'a [TestCase], support: &[&'a Unit]) -> Vec<BTreeSet<&'a str>> {
    let helpers: BTreeSet<&str> = support.iter().map(|u| u.short_name.as_str()).collect();
    let names: Vec<BTreeSet<&str>> = cases
        .iter()
        .map(|case| {
            case.subjects
                .iter()
                .map(String::as_str)
                .chain(
                    case.calls
                        .iter()
                        .map(String::as_str)
                        .filter(|c| helpers.contains(c)),
                )
                .collect()
        })
        .collect();
    let mut mentions = BTreeMap::<&str, usize>::new();
    for name in names.iter().flatten() {
        *mentions.entry(name).or_default() += 1;
    }
    let common = (cases.len() / 2).max(2);
    names
        .into_iter()
        .map(|set| set.into_iter().filter(|n| mentions[n] <= common).collect())
        .collect()
}

/// Positions merged into at most `MAX_GROUPS` sets, in order of each set's first position.
fn clusters(weights: Vec<Vec<u32>>) -> Vec<Vec<usize>> {
    let n = weights.len();
    if n == 0 {
        return Vec::new();
    }
    let mut sets: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut links = weights;
    while let Some((a, b, average)) = strongest(&sets, &links)
        && average >= LINK
    {
        merge(&mut sets, &mut links, a, b);
    }
    let mut loose = attach_singletons(&mut sets, &mut links);
    cap_groups(&mut sets, &mut links, &mut loose);
    if !loose.is_empty() {
        sets.push(loose);
    }
    for set in &mut sets {
        set.sort_unstable();
    }
    sets.sort_by_key(|set| set[0]);
    sets
}

/// Connected singletons join the cluster they link to most; members with no
/// link at all are returned to share one group.
fn attach_singletons(sets: &mut Vec<Vec<usize>>, links: &mut Vec<Vec<u32>>) -> Vec<usize> {
    let mut loose = Vec::new();
    while let Some(single) = sets.iter().position(|set| set.len() == 1) {
        if sets.len() == 1 {
            break;
        }
        let target = (0..sets.len()).filter(|&t| t != single).max_by(|&x, &y| {
            links[single][x]
                .cmp(&links[single][y])
                .then(sets[x].len().cmp(&sets[y].len()))
                .then(y.cmp(&x))
        });
        match target {
            Some(target) if links[single][target] > 0 => merge(sets, links, target, single),
            _ => {
                loose.push(sets[single][0]);
                remove(sets, links, single);
            }
        }
    }
    loose
}

/// Merge the most linked clusters, then fold the smallest into `loose`, until
/// at most `MAX_GROUPS` remain.
fn cap_groups(sets: &mut Vec<Vec<usize>>, links: &mut Vec<Vec<u32>>, loose: &mut Vec<usize>) {
    while sets.len() + usize::from(!loose.is_empty()) > MAX_GROUPS {
        match strongest(sets, links) {
            Some((a, b, average)) if average > 0.0 => merge(sets, links, a, b),
            _ => {
                let smallest = (0..sets.len())
                    .rev()
                    .min_by_key(|&i| sets[i].len())
                    .unwrap();
                loose.extend(sets[smallest].clone());
                remove(sets, links, smallest);
            }
        }
    }
}

/// The pair of clusters with the highest average link; ties keep the earliest pair.
fn strongest(sets: &[Vec<usize>], links: &[Vec<u32>]) -> Option<(usize, usize, f64)> {
    let mut best: Option<(usize, usize, f64)> = None;
    for a in 0..sets.len() {
        for b in a + 1..sets.len() {
            let average = f64::from(links[a][b]) / (sets[a].len() * sets[b].len()) as f64;
            if best.is_none_or(|(_, _, current)| average > current) {
                best = Some((a, b, average));
            }
        }
    }
    best
}

fn merge(sets: &mut Vec<Vec<usize>>, links: &mut Vec<Vec<u32>>, into: usize, from: usize) {
    let moved = sets[from].clone();
    sets[into].extend(moved);
    let moved = links[from].clone();
    for (slot, add) in links[into].iter_mut().zip(moved) {
        *slot += add;
    }
    links[into][into] = 0;
    let merged = links[into].clone();
    for (row, value) in links.iter_mut().zip(merged) {
        row[into] = value;
    }
    remove(sets, links, from);
}

fn remove(sets: &mut Vec<Vec<usize>>, links: &mut Vec<Vec<u32>>, index: usize) {
    sets.remove(index);
    links.remove(index);
    for row in links.iter_mut() {
        row.remove(index);
    }
}

fn weights(units: &[Unit], members: &[usize], imports: &BTreeSet<String>) -> Vec<Vec<u32>> {
    let n = members.len();
    let names = linking_names(units, members, imports);
    let mut weights = vec![vec![0u32; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (&units[members[i]], &units[members[j]]);
            let shared = names[i].intersection(&names[j]).count().min(2) as u32;
            let weight = link(a, b) + shared;
            weights[i][j] = weight;
            weights[j][i] = weight;
        }
    }
    weights
}

/// Per member, the names it mentions that link members: only names this file
/// declares or imports, and not a name most members mention.
fn linking_names<'a>(
    units: &'a [Unit],
    members: &[usize],
    imports: &'a BTreeSet<String>,
) -> Vec<BTreeSet<&'a str>> {
    let declared: BTreeSet<&str> = units
        .iter()
        .filter(|u| u.kind == Kind::Type)
        .map(|u| u.short_name.as_str())
        .chain(
            units
                .iter()
                .map(|u| u.owner.as_str())
                .filter(|o| !o.is_empty()),
        )
        .chain(imports.iter().map(String::as_str))
        .collect();
    let mut mentions = BTreeMap::<&str, usize>::new();
    for &m in members {
        for name in &units[m].refs {
            *mentions.entry(name.as_str()).or_default() += 1;
        }
    }
    let common = (members.len() / 2).max(2);
    members
        .iter()
        .map(|&m| {
            units[m]
                .refs
                .iter()
                .map(String::as_str)
                .filter(|name| declared.contains(name) && mentions[name] <= common)
                .collect()
        })
        .collect()
}

/// A call (or constructing the type that owns a method) links strongly; a shared owner less.
fn link(a: &Unit, b: &Unit) -> u32 {
    let calls = |x: &Unit, y: &Unit| {
        x.calls.contains(&y.short_name) || !y.owner.is_empty() && x.calls.contains(&y.owner)
    };
    let mut weight = 0;
    if calls(a, b) || calls(b, a) {
        weight += CALL_WEIGHT;
    }
    if !a.owner.is_empty() && a.owner == b.owner {
        weight += OWNER_WEIGHT;
    }
    weight
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn grouped(source: &str) -> Vec<Vec<String>> {
        let file = super::super::units::parse(Path::new("lib.rs"), source).unwrap();
        let members: Vec<usize> = (0..file.units.len()).collect();
        groups(&file.units, &members, &file.imports)
            .into_iter()
            .map(|g| {
                g.members
                    .iter()
                    .map(|&m| file.units[m].name.clone())
                    .collect()
            })
            .collect()
    }

    const TWO_CONCERNS: &str = "struct Cache { entries: Vec<u8> }\nimpl Cache {\n    fn get(&self) -> u8 { self.entries[0] }\n    fn put(&mut self, v: u8) { self.entries.push(v) }\n}\nfn warm(cache: &mut Cache) { cache.put(1); }\n\nstruct Html { body: String }\nfn render(page: &Html) -> String { escape(&page.body) }\nfn escape(text: &str) -> String { text.replace('<', \"&lt;\") }\nfn page(body: String) -> Html { Html { body } }\n";

    #[test]
    fn groups_separate_unrelated_concerns_deterministically() {
        let first = grouped(TWO_CONCERNS);
        assert_eq!(
            first,
            [
                vec!["Cache", "Cache::get", "Cache::put", "warm"],
                vec!["Html", "render", "escape", "page"],
            ]
        );
        for _ in 0..5 {
            assert_eq!(grouped(TWO_CONCERNS), first);
        }
    }

    #[test]
    fn a_large_type_is_parted_by_its_calls_not_its_owner() {
        let scraper = [
            "fetched_attributes",
            "fetched_html",
            "fetched_pdf",
            "canonical_target",
        ];
        let mut source = String::from("struct Story { title: String }\nimpl Story {\n");
        for i in 0..9 {
            source.push_str(&format!(
                "    fn field{i}(&self) -> usize {{ self.title.len() + {i} }}\n"
            ));
        }
        for name in scraper {
            let calls: Vec<String> = scraper
                .iter()
                .filter(|other| **other != name)
                .map(|other| format!("self.{other}()"))
                .collect();
            source.push_str(&format!(
                "    fn {name}(&self) -> usize {{ {} }}\n",
                calls.join(" + ")
            ));
        }
        source.push_str("}\n");
        let file = super::super::units::parse(Path::new("story.rs"), &source).unwrap();
        let members: Vec<usize> = (0..file.units.len()).collect();
        let name = |m: &usize| file.units[*m].short_name.as_str();
        assert!(
            groups(&file.units, &members, &file.imports)
                .iter()
                .any(|g| g.members.iter().any(|m| name(m) == "field0")
                    && g.members.iter().any(|m| name(m) == scraper[0])),
            "a shared owner links the scraper to the fields"
        );
        let parts = parts(&file.units, &members, &file.imports);
        let scraping = parts
            .iter()
            .find(|p| p.iter().any(|m| name(m) == scraper[0]))
            .unwrap();
        assert_eq!(scraping.iter().map(name).collect::<Vec<_>>(), scraper);
    }

    #[test]
    fn name_words_split_humps_underscores_and_private_marks() {
        let words = |name: &str| name_words(name).into_iter().collect::<Vec<_>>();
        assert_eq!(
            words("fetchedAttributesHtml"),
            ["attributes", "fetched", "html"]
        );
        assert_eq!(words("#parse_UTF8_value"), ["parse", "utf8", "value"]);
        assert_eq!(words("to"), Vec::<String>::new());
    }

    #[test]
    fn constructing_a_class_links_to_its_methods() {
        let source = "export class GatewayError extends Error {\n  constructor(code: string) {\n    super(code)\n  }\n}\nexport function requireLive(at: number) {\n  if (Date.now() >= at) throw new GatewayError('EXPIRED')\n}\n";
        let file = super::super::units::parse(Path::new("errors.ts"), source).unwrap();
        let members: Vec<usize> = (0..file.units.len()).collect();
        assert_eq!(groups(&file.units, &members, &file.imports).len(), 1);
    }

    #[test]
    fn groups_are_capped_and_loose_members_are_gathered() {
        let mut source = String::new();
        for i in 0..9 {
            source.push_str(&format!(
                "struct T{i} {{ f{i}: u8 }}\nfn use{i}(v: T{i}) -> u8 {{ v.f{i} }}\n"
            ));
        }
        source.push_str("fn alone() {}\nfn lonely() {}\n");
        let groups = grouped(&source);
        assert_eq!(groups.len(), MAX_GROUPS);
        assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), 20);
        assert!(
            groups
                .iter()
                .any(|g| g.contains(&"alone".to_string()) && g.contains(&"lonely".to_string()))
        );
    }
}
