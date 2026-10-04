//! What a check of changed lines touched, and the shared-logic findings
//! whose copies it did not all touch: such a finding points at a copy the
//! change touched and names the untouched ones, so that a change fixes its
//! own copy and leaves the others alone.
use super::compose::reorder;
use crate::{
    catalog,
    inventory::Input,
    revision::FileChange,
    schema::{FileResult, Finding, Location, Untouched},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// The files a check of changed lines judged, with the lines its change
/// touched in each, or none for a file it judged whole, such as an added one.
pub(crate) struct Changed<'a>(BTreeMap<&'a Path, Option<&'a FileChange>>);

impl<'a> Changed<'a> {
    pub fn of(inputs: &'a [Input]) -> Self {
        Self(
            inputs
                .iter()
                .map(|input| (input.result.path.as_path(), input.changed.as_ref()))
                .collect(),
        )
    }

    /// Whether the change touched lines at `location`. A copy in a file the
    /// check did not select, such as explicit context, is unchanged.
    pub fn touches(&self, location: &Location) -> bool {
        match self.0.get(location.path.as_path()) {
            Some(Some(change)) => change.lines.touch(location.start_line, location.end_line),
            Some(None) => true,
            None => false,
        }
    }

    /// Whether the change edits the file at `path`.
    fn edits(&self, path: &Path) -> bool {
        self.0.contains_key(path)
    }
}

/// Point each shared-logic finding whose copies the change did not all
/// touch at the first copy it did, in that copy's file, and name the
/// untouched copies in its message and `untouched`. The fingerprint stays
/// the one its pair was found with, so a baseline still accepts it.
pub(crate) fn anchor(changed: &Changed<'_>, files: &mut [FileResult]) {
    let rule = catalog::id(catalog::SHARED_LOGIC);
    let at: BTreeMap<PathBuf, usize> = files
        .iter()
        .enumerate()
        .map(|(f, file)| (file.path.clone(), f))
        .collect();
    let mut moves = Vec::new();
    for (f, file) in files.iter_mut().enumerate() {
        for (i, finding) in file.findings.iter_mut().enumerate() {
            if finding.rule != rule || !finding.untouched.is_empty() {
                continue;
            }
            let (touched, untouched): (Vec<Location>, Vec<Location>) = finding
                .locations
                .iter()
                .cloned()
                .partition(|l| changed.touches(l));
            // A touched copy is in a file the check judged.
            let Some(&to) = touched.first().and_then(|own| at.get(&own.path)) else {
                continue;
            };
            if untouched.is_empty() {
                continue;
            }
            if to != f {
                moves.push((f, i, to));
            }
            reword(finding, touched, untouched, changed);
        }
    }
    move_findings(files, moves);
}

fn reword(
    finding: &mut Finding,
    touched: Vec<Location>,
    untouched: Vec<Location>,
    changed: &Changed<'_>,
) {
    finding.untouched = untouched
        .into_iter()
        .map(|location| Untouched {
            file_changed: changed.edits(&location.path),
            location,
        })
        .collect();
    let (message, action) = super::wording::untouched_wording(&touched, &finding.untouched);
    finding.message = message;
    finding.action = action.into();
    finding.line = touched[0].start_line;
    finding.locations = touched;
    finding
        .locations
        .extend(finding.untouched.iter().map(|u| u.location.clone()));
}

/// Move each finding `(file, index, to)` to the file `to`, then restore
/// the order and status of the files whose findings changed.
fn move_findings(files: &mut [FileResult], mut moves: Vec<(usize, usize, usize)>) {
    // From the last index down, so earlier indexes stay valid.
    moves.sort_by_key(|&(from, index, _)| std::cmp::Reverse((from, index)));
    let mut changed = BTreeSet::new();
    for (from, index, to) in moves {
        let finding = files[from].findings.remove(index);
        files[to].findings.push(finding);
        changed.extend([from, to]);
    }
    for f in changed {
        reorder(&mut files[f]);
    }
}
