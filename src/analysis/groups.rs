//! Member groups for file organization: a per-file graph of calls, shared
//! owners and shared file-declared types or imports, merged deterministically
//! by average linkage. Label propagation was tried first and let generic
//! shared names pull every member into one group.
use super::units::{Kind, Unit};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_GROUPS: usize = 6;
/// Clusters merge while the average weight between their members reaches one
/// shared name; a call alone links two members strongly.
const LINK: f64 = 1.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// `G1`, `G2`… in order of each group's first member.
    pub id: String,
    /// Indexes into the unit list that was grouped.
    pub members: Vec<usize>,
}

/// Group the selected `members` (indexes into `units`). Deterministic for the same input.
pub fn groups(units: &[Unit], members: &[usize], imports: &BTreeSet<String>) -> Vec<Group> {
    let n = members.len();
    if n == 0 {
        return Vec::new();
    }
    let weights = weights(units, members, imports);
    let mut sets: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    let mut links = weights.clone();
    while let Some((a, b, average)) = strongest(&sets, &links)
        && average >= LINK
    {
        merge(&mut sets, &mut links, a, b);
    }
    // Connected singletons join the cluster they link to most; members with
    // no link at all share one group.
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
            Some(target) if links[single][target] > 0 => {
                merge(&mut sets, &mut links, target, single)
            }
            _ => {
                loose.push(sets[single][0]);
                remove(&mut sets, &mut links, single);
            }
        }
    }
    while sets.len() + usize::from(!loose.is_empty()) > MAX_GROUPS {
        match strongest(&sets, &links) {
            Some((a, b, average)) if average > 0.0 => merge(&mut sets, &mut links, a, b),
            _ => {
                // No links remain: fold the smallest set into the loose members.
                let smallest = (0..sets.len())
                    .rev()
                    .min_by_key(|&i| sets[i].len())
                    .unwrap();
                loose.extend(sets[smallest].clone());
                remove(&mut sets, &mut links, smallest);
            }
        }
    }
    if !loose.is_empty() {
        sets.push(loose);
    }
    for set in &mut sets {
        set.sort_unstable();
    }
    sets.sort_by_key(|set| set[0]);
    sets.into_iter()
        .enumerate()
        .map(|(i, set)| Group {
            id: format!("G{}", i + 1),
            members: set.into_iter().map(|m| members[m]).collect(),
        })
        .collect()
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
    // Only names this file declares or imports say which members belong together,
    // and a name most members mention says nothing.
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
    let common = (n / 2).max(2);
    let names: Vec<BTreeSet<&str>> = members
        .iter()
        .map(|&m| {
            units[m]
                .refs
                .iter()
                .map(String::as_str)
                .filter(|name| declared.contains(name) && mentions[name] <= common)
                .collect()
        })
        .collect();
    let mut weights = vec![vec![0u32; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let (a, b) = (&units[members[i]], &units[members[j]]);
            let mut weight = 0;
            // Calling a function, or constructing the type that owns a method.
            let calls = |x: &Unit, y: &Unit| {
                x.calls.contains(&y.short_name) || !y.owner.is_empty() && x.calls.contains(&y.owner)
            };
            if calls(a, b) || calls(b, a) {
                weight += 3;
            }
            if !a.owner.is_empty() && a.owner == b.owner {
                weight += 2;
            }
            weight += names[i].intersection(&names[j]).count().min(2) as u32;
            weights[i][j] = weight;
            weights[j][i] = weight;
        }
    }
    weights
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
