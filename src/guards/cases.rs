//! Test cases a change removed, and those whose assertions it changed: the
//! evidence for the question whether a test now checks less.
use super::markers;
use crate::analysis::{test_map::TestCase, units::Unit};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Helpers sent with a rewritten test, at most.
const MAX_HELPERS: usize = 3;
/// A helper's source is cut at this many bytes.
const HELPER_BYTES: usize = 4000;

/// A test in both versions of a change whose assertion lines lost or
/// changed a line, with its source before and after.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChangedTest {
    pub path: PathBuf,
    /// The hash of the file's current text, so a request about it is sent
    /// only while the file is unchanged.
    pub source_hash: String,
    pub line: usize,
    pub name: String,
    pub before: String,
    pub after: String,
    /// The functions of the file the new version calls and the old one did
    /// not: an assertion moved into a helper is still checked there.
    pub helpers: Vec<Helper>,
}

/// A function of a test's file, by name, with its source.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Helper {
    pub name: String,
    pub source: String,
}

/// What a change did to one file's tests.
#[derive(Debug, Default, PartialEq)]
pub(super) struct TestChanges {
    /// Names of the previous version's tests the current one lacks.
    pub removed: Vec<String>,
    /// Names of the current version's tests the previous one lacks.
    pub added: Vec<String>,
    pub changed: Vec<ChangedTest>,
}

/// Compare the test cases `old` of the text `before` with the cases `new`
/// of `after`, the file's current text at `path`.
pub(super) fn compare(
    path: &Path,
    (old, before): (&[TestCase], &str),
    (new, after): (&[TestCase], &str),
) -> TestChanges {
    let mut remaining: BTreeMap<&str, Vec<&TestCase>> = BTreeMap::new();
    for case in new {
        remaining.entry(&case.name).or_default().push(case);
    }
    let mut changes = TestChanges::default();
    let source_hash = crate::schema::hash(after.as_bytes());
    let mut units = None;
    for case in old {
        let Some(now) = remaining.get_mut(case.name.as_str()).and_then(Vec::pop) else {
            changes.removed.push(case.name.clone());
            continue;
        };
        let (was, is) = (case.source(before), now.source(after));
        if was != is && lost_assertion(was, is) {
            let units = units.get_or_insert_with(|| {
                crate::analysis::units::parse(path, after).map_or_else(|_| Vec::new(), |f| f.units)
            });
            changes.changed.push(ChangedTest {
                path: path.to_path_buf(),
                source_hash: source_hash.clone(),
                line: now.line,
                name: now.name.clone(),
                before: was.to_string(),
                after: is.to_string(),
                helpers: helpers(units, after, &now.calls - &case.calls),
            });
        }
    }
    changes.added = remaining
        .into_values()
        .flatten()
        .map(|case| case.name.clone())
        .collect();
    changes
}

/// The functions among `units` of the file `source` whose names `calls`
/// holds, cut to size.
fn helpers(units: &[Unit], source: &str, calls: BTreeSet<String>) -> Vec<Helper> {
    units
        .iter()
        .filter(|unit| calls.contains(&unit.short_name))
        .take(MAX_HELPERS)
        .map(|unit| {
            let text = unit.source(source);
            let cut = (0..=HELPER_BYTES.min(text.len()))
                .rev()
                .find(|&at| text.is_char_boundary(at))
                .unwrap_or(0);
            Helper {
                name: unit.name.clone(),
                source: text[..cut].to_string(),
            }
        })
        .collect()
}

/// Whether some assertion line of `before` is missing from `after`, as
/// trimmed text: an assertion removed or rewritten, not only one added.
fn lost_assertion(before: &str, after: &str) -> bool {
    let mut kept: BTreeMap<&str, usize> = BTreeMap::new();
    for line in markers::assertion_lines(after) {
        *kept.entry(line).or_default() += 1;
    }
    markers::assertion_lines(before)
        .into_iter()
        .any(|line| match kept.get_mut(line) {
            Some(n) if *n > 0 => {
                *n -= 1;
                false
            }
            _ => true,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::test_map::cases;

    const BEFORE: &str = "#[test]\nfn adds() {\n    assert_eq!(add(1, 2), 3);\n}\n\n#[test]\nfn empty() {\n    assert_eq!(add(0, 0), 0);\n}\n\n#[test]\nfn negative() {\n    assert_eq!(add(-1, -2), -3);\n}\n";

    fn compared(path: &Path, before: &str, after: &str) -> TestChanges {
        let (old, new) = (cases(path, before).unwrap(), cases(path, after).unwrap());
        compare(path, (&old, before), (&new, after))
    }

    #[test]
    fn removed_tests_and_weaker_assertions_are_told_apart_from_additions() {
        let after = "#[test]\nfn adds() {\n    assert!(add(1, 2) > 0);\n}\n\n#[test]\nfn empty() {\n    assert_eq!(add(0, 0), 0);\n    assert_eq!(add(0, 1), 1);\n}\n\n#[test]\nfn large() {\n    assert_eq!(add(1 << 30, 1), (1 << 30) + 1);\n}\n";
        let path = Path::new("tests/add.rs");
        let changes = compared(path, BEFORE, after);
        assert_eq!(changes.removed, ["negative"]);
        assert_eq!(changes.added, ["large"]);
        assert_eq!(changes.changed.len(), 1, "an added assertion checks more");
        let changed = &changes.changed[0];
        assert_eq!((changed.name.as_str(), changed.line), ("adds", 2));
        assert!(changed.before.contains("assert_eq!(add(1, 2), 3)"));
        assert!(changed.after.contains("assert!(add(1, 2) > 0)"));
        assert_eq!(
            compared(path, BEFORE, BEFORE),
            TestChanges::default(),
            "nothing changed"
        );
    }

    #[test]
    fn a_rewritten_test_carries_the_helpers_it_newly_calls() {
        let helper = "fn equals_three(total: i32) {\n    assert_eq!(total, 3);\n}\n\n";
        let before = format!(
            "{helper}#[test]\nfn adds() {{\n    let total = add(1, 2);\n    assert_eq!(total, 3);\n}}\n"
        );
        let after = format!(
            "{helper}#[test]\nfn adds() {{\n    let total = add(1, 2);\n    equals_three(total);\n}}\n"
        );
        let changes = compared(Path::new("tests/add.rs"), &before, &after);
        let helpers = &changes.changed[0].helpers;
        assert_eq!(
            helpers,
            &[Helper {
                name: "equals_three".into(),
                source: helper.trim_end().into()
            }],
            "`add` was called before too"
        );
    }
}
