//! Evidence units: small typed requests about one function group, one file
//! outline, one candidate pair or a few tests. Code builds the evidence,
//! Jev answers short literal questions, and `compose` turns answers into results.
mod answers;
pub mod compose;
mod duplicates;
mod evidence;
mod functions;
pub(crate) mod outline;
mod plan;
pub mod questions;
mod test_units;
mod wording;

use answers::Questions;
pub use answers::{Asked, locates, rechecks, record};
use evidence::{FileContext, compact, identity, pack, request, unique_ids};
use plan::Scope;
pub use plan::plan;

use crate::schema::Location;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

const PACK_ITEMS: usize = 8;
/// Tests are sent one per request: seven unrelated tests in the same state
/// left about 40% more test-value questions undecided.
const TEST_PACK_ITEMS: usize = 1;

pub struct Planned {
    pub owner: usize,
    pub request: Value,
    pub asked: Asked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Judged,
    /// Below the minimum body size; never clear.
    TooSmall,
    /// The unit alone exceeds the provider limit, so it was not sent.
    NeedsContext,
}

#[derive(Clone, Debug)]
pub struct GroupInfo {
    pub id: String,
    pub names: Vec<String>,
    pub locations: Vec<Location>,
    /// Other selected files that import this file and call a member of the group.
    pub users: BTreeSet<PathBuf>,
}

/// A top-level block of a function body, offered when locating a split.
#[derive(Clone, Debug)]
pub struct Block {
    pub id: String,
    pub location: Location,
}

#[derive(Clone, Debug)]
pub enum Detail {
    Function {
        blocks: Vec<Block>,
        /// The follow-up that asks which block to extract, sent only after the
        /// split question raises a review or consider.
        locate: Option<(Value, Asked)>,
    },
    Outline {
        groups: Vec<GroupInfo>,
    },
    Pair {
        differences: Vec<crate::analysis::clones::Difference>,
        /// Both copies sit in one test case: the remedy is a table of cases,
        /// not a shared implementation.
        within_test: bool,
        /// The owning copy is test code: shared steps belong in a fixture or helper.
        in_tests: bool,
        /// Every copy is inside a test case, where spelling out each case is idiomatic.
        in_cases: bool,
    },
    Test,
    TestPair {
        names: [String; 2],
        subject: String,
    },
}

#[derive(Clone, Debug)]
pub struct UnitPlan {
    pub rule: &'static str,
    /// Unique within the file; judgments refer to it.
    pub id: String,
    pub name: String,
    pub presence: Presence,
    pub locations: Vec<Location>,
    pub quote: Option<String>,
    pub lines: usize,
    /// Identity for the finding fingerprint: survives moves and unrelated edits.
    pub identity: String,
    pub detail: Detail,
    pub recheck: Option<(Value, Asked)>,
}

#[derive(Clone, Debug, Default)]
pub struct FilePlan {
    pub path: PathBuf,
    /// Rules that apply to this file, with the candidates omitted by caps.
    pub rules: BTreeMap<&'static str, usize>,
    pub units: Vec<UnitPlan>,
}

#[derive(Default)]
pub struct Plan {
    pub files: BTreeMap<usize, FilePlan>,
    pub requests: Vec<Planned>,
    /// Files with no supported parser or with syntax errors, and why.
    pub skipped: BTreeMap<usize, String>,
}

#[cfg(test)]
mod tests;
