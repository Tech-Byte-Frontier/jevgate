//! A unit: a function, method, type or law of one file, with what the
//! rules read from it.
use crate::analysis::{bend, errors, literals, nesting, routes, sites};
use std::{collections::BTreeSet, ops::Range};

/// Bodies with fewer non-brace lines are too small to judge. They are never clear.
pub const MIN_BODY_LINES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Function,
    Method,
    Type,
    /// A Bend 2 law: a claim, a signature or a postulate, with the calls of
    /// its statement, which name the functions it is about.
    Law,
}

/// What a callable unit is to the rules that judge only running code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Role {
    /// Code that runs: every unit outside Bend 2, and most Bend 2 defs.
    #[default]
    Code,
    /// A Bend 2 proof of a law or lemma: its literals and calls state a
    /// property, and it never runs outside the checker.
    Proof,
    /// A Bend 2 def that computes a type (`-> Type`), such as a proposition.
    TypeLevel,
}

#[derive(Clone, Debug)]
pub struct Unit {
    /// `Owner::name` for methods; the bare name otherwise.
    pub name: String,
    pub short_name: String,
    pub owner: String,
    pub kind: Kind,
    /// Definition including leading documentation, comments and attributes.
    pub span: Range<usize>,
    pub line: usize,
    pub end_line: usize,
    pub body: Option<Range<usize>>,
    pub signature: String,
    pub doc: String,
    pub body_lines: usize,
    /// Deepest nesting of control flow in the body, and the longest chain of
    /// `else if`, `elif` or nested conditional-expression branches.
    pub nesting: usize,
    pub branch_chain: usize,
    /// Top-level statement blocks of the body, as byte ranges; empty when
    /// there is no choice of block to extract.
    pub blocks: Vec<Range<usize>>,
    /// Eligible literal values in the body, for hardcoded-value questions.
    pub literals: Vec<literals::Literal>,
    /// Calls, built text and field assignments, for locating security findings.
    pub sites: Vec<sites::Site>,
    /// Errors the body creates with their message arguments, for error-detail questions.
    pub errors: Vec<errors::CreatedError>,
    pub calls: BTreeSet<String>,
    /// Functions it passes by path without calling them, in Rust: a callback
    /// named as `compose::unconfirmed_units` or `Self::helper`. A file's
    /// callers count them as calls; nothing else reads them.
    pub passed: BTreeSet<String>,
    /// A Java `equals(Object)` or `hashCode()` override: boilerplate whose
    /// field-by-field copies and hash multipliers are the idiom, so it offers
    /// no copies or literal values to judge.
    pub equality: bool,
    /// Spring MVC routes a Java controller method maps, so tests that send
    /// requests to them are linked to it.
    pub routes: Vec<routes::Route>,
    /// Type, field and imported names this unit mentions, including its own name.
    pub refs: BTreeSet<String>,
    pub role: Role,
    /// A Bend 2 def that performs effects: it returns `IO`, runs a `do IO`
    /// block or imports its host code.
    pub effects: bool,
    /// A Bend 2 def that joins text with `++`, as a request, query or
    /// markup is built before an effect sends it.
    pub joins_text: bool,
    /// What a Bend 2 law states, which tells a claim from a signature.
    pub statement: Option<bend::Statement>,
    /// Plain identifiers, used only while parsing to find functions passed by name.
    pub(super) mentions: BTreeSet<String>,
}

impl Unit {
    pub fn source<'a>(&self, source: &'a str) -> &'a str {
        &source[self.span.clone()]
    }

    /// Code whose values can reach another program, a log or a user: every
    /// callable outside Bend 2, and a Bend 2 def that runs and performs
    /// effects or builds text. Bend's other defs are pure: nothing reaches
    /// them from outside the program but through their callers, and they
    /// send, store and log nothing.
    pub fn reaches_out(&self, bend: bool) -> bool {
        self.callable() && (!bend || self.role == Role::Code && (self.effects || self.joins_text))
    }

    pub fn callable(&self) -> bool {
        !matches!(self.kind, Kind::Type | Kind::Law)
    }

    pub fn too_small(&self) -> bool {
        self.body_lines < MIN_BODY_LINES
    }

    /// Control flow deep or long enough that flattening it is worth asking about.
    pub fn deeply_nested(&self) -> bool {
        self.nesting >= nesting::DEEP_NESTING || self.branch_chain >= nesting::LONG_CHAIN
    }

    pub fn lines(&self) -> usize {
        self.end_line + 1 - self.line
    }

    pub fn overlaps(&self, lines: &Range<usize>) -> bool {
        self.line < lines.end && lines.start <= self.end_line
    }
}
