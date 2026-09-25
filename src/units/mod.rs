//! Evidence units: small typed requests about one function group, one file
//! outline, one candidate pair or a few tests. Code builds the evidence,
//! Jev answers short literal questions, and `compose` turns answers into results.
mod access;
mod answers;
mod comments;
pub mod compose;
mod documents;
mod drift;
mod duplicates;
mod evidence;
mod follow_ups;
mod functions;
pub mod grouping;
mod handlers;
mod hardcoded;
mod instructions;
mod nextjs;
mod outcome;
pub(crate) mod outline;
mod plan;
pub mod questions;
mod security;
mod spacetimedb;
mod sveltekit;
mod test_units;
mod wording;
mod workflows;

use answers::Questions;
pub use answers::{Asked, record};
use evidence::{FileContext, compact, identity, pack, pack_runs, request, unique_ids};
pub use follow_ups::{doc_checks, kinds, locates, rechecks, settles, traces};
use plan::Scope;
pub use plan::plan;
pub use spacetimedb::spacetimedb_module;

use crate::schema::Location;
use serde_json::Value;
use std::{collections::BTreeMap, path::PathBuf};

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
}

/// A top-level block of a function body, offered when locating a split.
#[derive(Clone, Debug)]
pub struct Block {
    pub id: String,
    pub location: Location,
}

/// A settle follow-up of a security unit: the Choice it asks, and its request.
#[derive(Clone, Debug)]
pub struct Settle {
    /// The Choice's question id, which `security::SETTLES` maps to the checks
    /// it settles and the options that clear them.
    pub question: &'static str,
    pub request: (Value, Asked),
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
        /// A test file's cases rather than application members.
        tests: bool,
        groups: Vec<GroupInfo>,
        /// What kind of file it is, asked after a recheck that stays undecided.
        kind: Option<(Value, Asked)>,
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
    /// A function and the literal values it uses.
    Values {
        values: Vec<String>,
        /// Its distinct values, whose ids `v0`, `v1`, ... the locate follow-up
        /// chooses among; that follow-up is sent only after a review or consider.
        choices: Vec<String>,
        locate: Option<(Value, Asked)>,
    },
    /// A comment of application code and the unit it documents or sits in.
    Comment {
        /// The name of that unit, or `top-level code`: a finding lists the
        /// comments of one unit together.
        owner: String,
        /// Documentation of a declaration or file, which a documentation
        /// tool may render even when it repeats the signature.
        documentation: bool,
        /// What kind of comment it is, asked when its questions stay undecided.
        kind: Option<(Value, Asked)>,
    },
    /// A file's module-level constants and the literal values they hold.
    Constants {
        values: Vec<String>,
    },
    /// A security unit: its statements as sites for locating a finding, and
    /// the trace follow-up sent when presence is not clear.
    Security {
        sites: Vec<Block>,
        /// The message argument of each error it creates, by position (`m0`…),
        /// for sensitive-data units.
        messages: Vec<String>,
        trace: Option<(Value, Asked)>,
        /// One Choice per kind of check that can stay undecided after the
        /// trace and recheck, such as where its URLs come from or its output
        /// goes; each is asked only while its checks are undecided.
        settles: Vec<Settle>,
        /// Django code, asked the Django checks: a weak setting must be
        /// named by one of them.
        django: bool,
    },
    /// A large document judged by its outline, with its top-level parts
    /// and the follow-up that locates a split.
    Document {
        parts: Vec<Block>,
        locate: Option<(Value, Asked)>,
        /// What kind of document it is, asked when the split stays undecided.
        kind: Option<(Value, Asked)>,
    },
    /// A document whose release is tagged or whose named paths were deleted:
    /// the facts a finished plan finding cites.
    Plan {
        facts: Vec<String>,
    },
    /// A section naming paths or scripts the repository lacks, and the check
    /// sent unless its document is a finished plan.
    Stale {
        missing: Vec<String>,
        check: Option<(Value, Asked)>,
        /// What the section treats the missing names as, asked when the
        /// check stays undecided.
        settle: Option<(Value, Asked)>,
    },
    /// A candidate pair of sections, the other in `other`, and the check
    /// sent unless either document is a finished plan.
    DocPair {
        other: Location,
        check: Option<(Value, Asked)>,
        /// How the two sections relate, asked when the check stays undecided.
        settle: Option<(Value, Asked)>,
    },
    /// A heading section of an agent instruction file.
    Section {
        /// Estimated tokens of the section's text.
        tokens: usize,
        /// Which harnesses load the file and when.
        loaded: String,
    },
    /// A web framework's error handler and how the program registers it.
    Handler {
        registered: String,
    },
    /// A policy, SECURITY DEFINER function or grant in its final state.
    Access(Access),
    /// A workflow job and the expressions its `run` scripts hold.
    Job {
        expressions: Vec<String>,
    },
    Test,
    TestPair {
        names: [String; 2],
        subject: String,
        /// Whether the two tests read the same apart from their names, in
        /// the same group.
        identical: bool,
        /// Whether their file's tests can be parameterized.
        table: bool,
        /// Tests in different groups whose setup is not sent (outside Ruby):
        /// each group's `before` hook may build a different case.
        unseen_setup: bool,
    },
}

/// What an access-control unit holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Policy {
        table: String,
    },
    Definer,
    Grant,
    /// A SpacetimeDB public table.
    Table,
    /// A SpacetimeDB view.
    View,
    /// A SpacetimeDB reducer.
    Reducer,
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
