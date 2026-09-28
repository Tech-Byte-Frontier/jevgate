//! Evidence units: small typed requests about one function group, one file
//! outline, one candidate pair or a few tests. Code builds the evidence,
//! Jev answers short literal questions, and `compose` turns answers into results.
mod access;
mod answers;
mod client_app;
mod comments;
pub mod compose;
mod documents;
mod drift;
mod duplicates;
mod evidence;
mod follow_ups;
mod functions;
mod graphql;
pub mod grouping;
mod guards;
mod handlers;
mod hardcoded;
mod instructions;
mod laws;
mod nextjs;
mod outcome;
pub(crate) mod outline;
mod packs;
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
pub use follow_ups::{doc_checks, kinds, locates, parts, rechecks, settles, traces, value_kinds};
pub use guards::{Steering, weaker_answer, weaker_request};
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

/// A candidate part of a long file and the follow-up that asks whether it
/// does a job of its own, sent only when the outline raised no finding.
#[derive(Clone, Debug)]
pub struct Part {
    /// The ids its answers are recorded under, from `outline::PART_QUESTIONS`.
    pub questions: (&'static str, &'static str),
    pub names: Vec<String>,
    pub locations: Vec<Location>,
    /// The lines of its members.
    pub lines: usize,
    pub follow_up: FollowUp,
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
    pub request: FollowUp,
}

/// A follow-up request, kept as its JSON text until it is asked: most are
/// never sent, and held as JSON values, the traces, rechecks and settles of
/// laravel/framework's security units took about 2 GB while planning.
/// `serde_json` reads the text back to the same value, so the request, and
/// the answer cache it keys, do not change.
#[derive(Clone, Debug)]
pub struct FollowUp {
    request: Box<str>,
    pub asked: Asked,
}

impl FollowUp {
    pub fn request(&self) -> Value {
        serde_json::from_str(&self.request).expect("a follow-up is JSON it wrote itself")
    }

    /// The request, planned for the file at `owner`.
    pub fn planned(&self, owner: usize) -> Planned {
        Planned {
            owner,
            request: self.request(),
            asked: self.asked.clone(),
        }
    }
}

impl From<(Value, Asked)> for FollowUp {
    fn from((request, asked): (Value, Asked)) -> Self {
        Self {
            request: request.to_string().into_boxed_str(),
            asked,
        }
    }
}

/// The Choices a security unit's finding is asked after it is raised, each
/// only for the findings it serves.
#[derive(Clone, Debug, Default)]
pub struct Confirms {
    /// For injection, what the values it places can hold, asked only after a
    /// consider that rests on its parameters.
    pub values: Option<FollowUp>,
    /// For injection, what the values of a path, markup or redirect finding
    /// can hold or where they lead, asked only after a finding whose one
    /// concern is one of those.
    pub checked: Option<FollowUp>,
    /// For injection, what the values of an SQL, command or code finding hold
    /// where they enter it, asked only after a finding whose one concern is
    /// one of those.
    pub queried: Option<FollowUp>,
    /// For weak settings, what the HTML written without escaping holds, asked
    /// only after a finding its escaping check raised.
    pub rendered: Option<FollowUp>,
    /// For sensitive data, who reads its error text, asked only after a
    /// finding its error-detail checks raised.
    pub readers: Option<FollowUp>,
    /// For sensitive data, when its log line runs, asked only after a finding
    /// its log checks raised.
    pub logging: Option<FollowUp>,
}

impl Confirms {
    fn all(&self) -> impl Iterator<Item = &FollowUp> {
        [
            &self.values,
            &self.checked,
            &self.queried,
            &self.rendered,
            &self.readers,
            &self.logging,
        ]
        .into_iter()
        .flatten()
    }
}

#[derive(Clone, Debug)]
pub enum Detail {
    Function {
        blocks: Vec<Block>,
        /// The follow-up that asks which block to extract, sent only after the
        /// split question raises a review or consider.
        locate: Option<FollowUp>,
    },
    Outline {
        /// A test file's cases rather than application members.
        tests: bool,
        groups: Vec<GroupInfo>,
        /// How many members the outline lists.
        members: usize,
        /// The section rules of a Bend 2 file (`# ----`, `# === Title ===`):
        /// the parts its author laid it out in. Zero for other languages.
        sections: usize,
        /// What kind of file it is, asked after a recheck that stays undecided.
        kind: Option<FollowUp>,
        /// The candidate parts of a long application file, each asked
        /// whether it does a job of its own once the outline stays without
        /// a finding.
        parts: Vec<Part>,
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
        /// Whether each choice's text is not written exactly once in the file.
        repeated: Vec<bool>,
        locate: Option<FollowUp>,
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
        kind: Option<FollowUp>,
    },
    /// A file's module-level constants and the literal values they hold.
    Constants {
        values: Vec<String>,
        /// Which constant a review or consider is about, asked after it;
        /// its options are the unit's locations, one per constant, in order.
        locate: Option<FollowUp>,
    },
    /// A security unit: its statements as sites for locating a finding, and
    /// the trace follow-up sent when presence is not clear.
    Security {
        sites: Vec<Block>,
        /// The message argument of each error it creates, by position (`m0`…),
        /// for sensitive-data units.
        messages: Vec<String>,
        trace: Option<FollowUp>,
        /// One Choice per kind of check that can stay undecided after the
        /// trace and recheck, such as where its URLs come from or its output
        /// goes; each is asked only while its checks are undecided.
        settles: Vec<Settle>,
        /// The Choices asked after a finding, each only for the findings it
        /// serves.
        confirms: Box<Confirms>,
        /// Django code, asked the Django checks: a weak setting must be
        /// named by one of them.
        django: bool,
        /// Code at a test path, such as a test app's settings or models.
        test_path: bool,
    },
    /// A large document judged by its outline, with its top-level parts
    /// and the follow-up that locates a split.
    Document {
        parts: Vec<Block>,
        locate: Option<FollowUp>,
        /// What kind of document it is, asked when the split stays undecided.
        kind: Option<FollowUp>,
    },
    /// A document whose release is tagged or whose named paths were deleted:
    /// the facts a finished plan finding cites.
    Plan { facts: Vec<String> },
    /// A section naming paths or scripts the repository lacks, and the check
    /// sent unless its document is a finished plan.
    Stale {
        missing: Vec<String>,
        check: Option<FollowUp>,
        /// What the section treats the missing names as, asked when the
        /// check stays undecided.
        settle: Option<FollowUp>,
    },
    /// A candidate pair of sections, the other in `other`, and the check
    /// sent unless either document is a finished plan.
    DocPair {
        other: Location,
        check: Option<FollowUp>,
        /// How the two sections relate, asked when the check stays undecided.
        settle: Option<FollowUp>,
        /// The two documents are written for readers of different languages,
        /// by their paths' locales or their scripts.
        translated: bool,
    },
    /// A heading section of an agent instruction file.
    Section {
        /// Estimated tokens of the section's text.
        tokens: usize,
        /// Which harnesses load the file and when.
        loaded: String,
    },
    /// A web framework's error handler and how the program registers it.
    Handler { registered: String },
    /// A policy, SECURITY DEFINER function or grant in its final state.
    Access(Access),
    /// A workflow job and the expressions its `run` scripts hold.
    Job { expressions: Vec<String> },
    /// A Bend 2 claim with the comment above it.
    Law,
    Test {
        /// What its assertions read, asked with the code under test after its
        /// first answer says it asserts internal details.
        confirm: Option<FollowUp>,
    },
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
        /// Outside Ruby, whether each test checks something the other does
        /// not, asked only of a pair that reached a review.
        confirm: Option<FollowUp>,
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
    pub recheck: Option<FollowUp>,
}

impl UnitPlan {
    /// Every follow-up planned for the unit, asked or not: where an answer
    /// given after the first pass was asked.
    fn follow_ups(&self) -> impl Iterator<Item = &FollowUp> {
        let planned: Vec<&FollowUp> = match &self.detail {
            Detail::Function { locate, .. }
            | Detail::Values { locate, .. }
            | Detail::Constants { locate, .. } => locate.iter().collect(),
            Detail::Outline { kind, parts, .. } => kind
                .iter()
                .chain(parts.iter().map(|part| &part.follow_up))
                .collect(),
            Detail::Comment { kind, .. } => kind.iter().collect(),
            Detail::Security {
                trace,
                settles,
                confirms,
                ..
            } => trace
                .iter()
                .chain(settles.iter().map(|settle| &settle.request))
                .chain(confirms.all())
                .collect(),
            Detail::Document { locate, kind, .. } => locate.iter().chain(kind).collect(),
            Detail::Stale { check, settle, .. } | Detail::DocPair { check, settle, .. } => {
                check.iter().chain(settle).collect()
            }
            Detail::Test { confirm } | Detail::TestPair { confirm, .. } => confirm.iter().collect(),
            Detail::Pair { .. }
            | Detail::Plan { .. }
            | Detail::Section { .. }
            | Detail::Handler { .. }
            | Detail::Access(_)
            | Detail::Job { .. }
            | Detail::Law => Vec::new(),
        };
        self.recheck.iter().chain(planned)
    }
}

#[derive(Clone, Debug, Default)]
pub struct FilePlan {
    pub path: PathBuf,
    /// Rules that apply to this file, with the candidates omitted by caps.
    pub rules: BTreeMap<&'static str, usize>,
    pub units: Vec<UnitPlan>,
    /// Comments and strings addressed to a reviewer that some request of
    /// the file sends, each asked whether it is written to steer the reviewer.
    pub steering: Vec<Steering>,
}

impl UnitPlan {
    /// Too large to send even alone: it needs context, and none of its
    /// follow-ups is asked.
    fn unsent(&mut self) {
        self.presence = Presence::NeedsContext;
        self.recheck = None;
        match &mut self.detail {
            Detail::Function { blocks, locate } => {
                blocks.clear();
                *locate = None;
            }
            Detail::Values { locate, .. } => *locate = None,
            Detail::Security {
                trace,
                settles,
                confirms,
                ..
            } => {
                *trace = None;
                settles.clear();
                **confirms = Confirms::default();
            }
            _ => {}
        }
    }
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
