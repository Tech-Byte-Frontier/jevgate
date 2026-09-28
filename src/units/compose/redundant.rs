//! Redundant tests: pairs linked into groups of three or more, and
//! shared-logic findings that repeat what a redundancy finding says.
use super::*;

/// A shared-logic finding whose every copy lies inside tests a redundancy
/// finding already names says the same thing twice: on sqlite-utils, 12
/// test pairs were reported by both rules. The redundancy finding stays,
/// since it says which test to merge or delete.
pub(super) fn drop_copies_of_redundant_tests(findings: &mut Vec<Finding>) {
    let tests: Vec<crate::schema::Location> = findings
        .iter()
        .filter(|f| f.rule == catalog::id(catalog::TEST_REDUNDANCY))
        .flat_map(|f| f.locations.iter().cloned())
        .collect();
    let named = |l: &crate::schema::Location| {
        tests
            .iter()
            .any(|t| t.path == l.path && t.start_line <= l.start_line && l.end_line <= t.end_line)
    };
    findings.retain(|f| {
        f.rule != catalog::id(catalog::SHARED_LOGIC)
            || f.locations.is_empty()
            || !f.locations.iter().all(named)
    });
}

/// A redundant test pair, with the index of its finding (a note) when it
/// reached a consider, which a group of three or more tests reports instead.
pub(super) struct Redundant<'p> {
    pub(super) unit: &'p UnitPlan,
    pub(super) names: &'p [String; 2],
    pub(super) subject: &'p String,
    pub(super) p: f64,
    pub(super) finding: Option<usize>,
}

/// Tests linked by overlapping pairs on one subject, with where they are,
/// the lowest probability of their pairs, and their pairs' findings.
pub(super) struct Cluster<'a> {
    subject: &'a String,
    tests: BTreeSet<&'a String>,
    locations: Vec<crate::schema::Location>,
    p: f64,
    findings: Vec<Option<usize>>,
}

/// Three or more tests linked by overlapping pairs on one subject: the tests
/// a chain of such pairs connects. Two pairs of one subject that share no
/// test stay two pairs; grouped by subject alone, sinatra's pair of redirect
/// tests and pair of deny tests of `get` read as four overlapping tests.
/// Also returns the indices of the pair findings each group reports.
pub(super) fn over_tested(
    plan: &FilePlan,
    redundant: &[Redundant<'_>],
) -> (Vec<Finding>, BTreeSet<usize>) {
    let clusters = clusters(redundant);
    let grouped = clusters
        .iter()
        .flat_map(|c| c.findings.iter().flatten().copied())
        .collect();
    let groups = clusters
        .into_iter()
        .map(|cluster| group_finding(plan, cluster))
        .collect();
    (groups, grouped)
}

/// The clusters of three or more tests that overlapping pairs connect.
pub(super) fn clusters<'a>(redundant: &'a [Redundant<'_>]) -> Vec<Cluster<'a>> {
    let mut clusters: Vec<Cluster<'a>> = Vec::new();
    for pair in redundant {
        let mut joined = Cluster {
            subject: pair.subject,
            tests: BTreeSet::new(),
            locations: Vec::new(),
            p: pair.p,
            findings: vec![pair.finding],
        };
        let mut index = 0;
        while index < clusters.len() {
            let other = &clusters[index];
            if other.subject == pair.subject && pair.names.iter().any(|n| other.tests.contains(n)) {
                let other = clusters.remove(index);
                joined.tests.extend(other.tests);
                joined.locations.extend(other.locations);
                joined.p = joined.p.min(other.p);
                joined.findings.extend(other.findings);
            } else {
                index += 1;
            }
        }
        for (name, location) in pair.names.iter().zip(&pair.unit.locations) {
            if joined.tests.insert(name) {
                joined.locations.push(location.clone());
            }
        }
        clusters.push(joined);
    }
    clusters.sort_by(|a, b| (a.subject, &a.tests).cmp(&(b.subject, &b.tests)));
    clusters.retain(|c| c.tests.len() >= 3);
    clusters
}

/// The consider that names a cluster's tests.
pub(super) fn group_finding(plan: &FilePlan, cluster: Cluster<'_>) -> Finding {
    let Cluster {
        subject,
        tests,
        mut locations,
        p,
        ..
    } = cluster;
    locations.sort();
    let names: Vec<String> = tests.iter().map(|t| format!("`{t}`")).collect();
    let lines = locations
        .iter()
        .map(|l| l.end_line + 1 - l.start_line)
        .sum();
    let identity: Vec<&str> = std::iter::once(subject.as_str())
        .chain(tests.iter().map(|t| t.as_str()))
        .collect();
    Finding {
        rule: catalog::id(catalog::TEST_REDUNDANCY).into(),
        strength: Strength::Consider,
        line: locations.first().map_or(1, |l| l.start_line),
        message: format!(
            "{} tests of `{subject}` overlap: {}.",
            tests.len(),
            names.join(", ")
        ),
        action: "Consider one parameterized test for these cases".into(),
        symbol: Some(subject.clone()),
        rule_version: catalog::rule_version(catalog::TEST_REDUNDANCY).into(),
        concern_probability: p,
        locations,
        quote: None,
        category: None,
        values: Vec::new(),
        fingerprint: fingerprint(
            catalog::TEST_REDUNDANCY,
            plan,
            &crate::units::identity(&identity),
        ),
        rank: rank(p, lines),
        baselined: false,
        suppressed: None,
        gate: None,
        precision: None,
    }
}
