//! Which baseline entry accepts a finding: the one with its fingerprint, or
//! one written before its fingerprint changed, through the fingerprints it
//! had then or the members it is made of.
use super::Accepted;
use crate::{catalog, schema::Finding};
use std::collections::{BTreeMap, BTreeSet};

/// An outline whose member names overlap an accepted outline's this much
/// (shared names over all names) is the one accepted: a file that grew a
/// little stays accepted, and one that grew a lot is asked about again.
const SIMILAR_OUTLINE: f64 = 0.8;

/// A baseline's entries, indexed to find the one that accepts a finding.
pub(super) struct Entries {
    pub entries: Vec<Accepted>,
    by_fingerprint: BTreeMap<String, usize>,
    /// Shared-logic entries by each copy they name.
    by_copy: BTreeMap<String, Vec<usize>>,
}

/// How an entry accepts a finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Found {
    /// By the finding's fingerprint.
    Own(usize),
    /// By a fingerprint the finding had before, under JevGate 0.35 or
    /// before its file was renamed: the next baseline write rewrites it.
    Earlier(usize),
    /// A repeat whose every copy shared-logic entries accept, as when a
    /// copy was removed or a check saw only some copies; the first of them.
    Copies(usize),
}

impl Found {
    pub fn at(self) -> usize {
        match self {
            Self::Own(at) | Self::Earlier(at) | Self::Copies(at) => at,
        }
    }
}

impl Entries {
    pub fn new(entries: Vec<Accepted>) -> Self {
        let mut by_fingerprint = BTreeMap::new();
        let mut by_copy = BTreeMap::<String, Vec<usize>>::new();
        let shared = catalog::id(catalog::SHARED_LOGIC);
        for (at, entry) in entries.iter().enumerate() {
            by_fingerprint
                .entry(entry.fingerprint.clone())
                .or_insert(at);
            if entry.rule == shared {
                for member in &entry.members {
                    by_copy.entry(member.clone()).or_default().push(at);
                }
            }
        }
        Self {
            entries,
            by_fingerprint,
            by_copy,
        }
    }

    /// The entry that accepts `finding`, if one does. An outline's entry
    /// accepts it only while their member names are similar; a repeat is
    /// accepted while every copy is one an accepted repeat names, and asked
    /// about again when a copy no entry names joins it.
    pub fn find(&self, finding: &Finding) -> Option<Found> {
        if let Some(&at) = self.by_fingerprint.get(&finding.fingerprint)
            && similar(&self.entries[at].members, &finding.identity.members)
        {
            return Some(Found::Own(at));
        }
        if let Some(at) = finding
            .identity
            .earlier()
            .find_map(|earlier| self.by_fingerprint.get(earlier))
        {
            return Some(Found::Earlier(*at));
        }
        self.copies(finding).map(Found::Copies)
    }

    pub fn get(&self, found: Found) -> &Accepted {
        &self.entries[found.at()]
    }

    /// The first shared-logic entry naming a copy of `finding`, when the
    /// entries naming its copies name all of them.
    fn copies(&self, finding: &Finding) -> Option<usize> {
        let members = &finding.identity.members;
        if finding.rule != catalog::id(catalog::SHARED_LOGIC) || members.is_empty() {
            return None;
        }
        let naming: BTreeSet<usize> = members
            .iter()
            .filter_map(|member| self.by_copy.get(member))
            .flatten()
            .copied()
            .collect();
        let named: BTreeSet<&String> = naming
            .iter()
            .flat_map(|&at| &self.entries[at].members)
            .collect();
        members
            .iter()
            .all(|member| named.contains(member))
            .then(|| naming.first().copied())
            .flatten()
    }
}

/// Whether an entry's members and a finding's are similar enough for the
/// entry to accept it: always when either has none, as every finding but
/// an outline or a repeat, whose fingerprint already names its copies.
fn similar(accepted: &[String], found: &[String]) -> bool {
    if accepted.is_empty() || found.is_empty() {
        return true;
    }
    let accepted: BTreeSet<&String> = accepted.iter().collect();
    let found: BTreeSet<&String> = found.iter().collect();
    let shared = accepted.intersection(&found).count();
    let all = accepted.union(&found).count();
    shared as f64 >= SIMILAR_OUTLINE * all as f64
}

#[cfg(test)]
mod tests {
    use super::similar;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn an_outline_stays_similar_until_a_fifth_of_its_names_change() {
        let before = names(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let mut grown = before.clone();
        grown.push("i".into());
        assert!(similar(&before, &grown), "8 of 9 names shared");
        grown.extend(names(&["j", "k"]));
        assert!(!similar(&before, &grown), "8 of 11 names shared");
        assert!(similar(&[], &grown), "an entry without members");
    }
}
