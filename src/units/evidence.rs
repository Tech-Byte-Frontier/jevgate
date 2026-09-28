//! Request building shared by every planner: one file's facts, the request
//! envelope, packing, and stable identities.
use super::{Asked, PACK_ITEMS, Questions};
use crate::{schema::Location, token_budget::Limits};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

/// Packed requests stay well below the provider's state limit so each
/// question sees a small state (about six thousand tokens). The budget is in
/// bytes, not calibrated tokens, so packing and cache identity stay stable.
const PACK_BYTES: usize = 18_000;

/// Shared facts one file's planners need.
pub(super) struct FileContext<'a> {
    pub owner: usize,
    pub path: &'a Path,
    pub language: &'static str,
    pub source: &'a str,
    pub source_hash: &'a str,
    pub model: &'a str,
    pub budget: Limits<'a>,
    /// What a web framework makes of the file, such as a Next.js route
    /// handler or Server Actions module, sent beside its path.
    pub framework: Option<String>,
    /// The opening of the repository's README, sent only with the question
    /// who reads a program's error text.
    pub project: Option<&'a str>,
    /// With `--base` judging what the change touched, what it did to this
    /// file; none when the whole file is judged.
    pub changed: Option<&'a crate::revision::FileChange>,
}

impl<'a> FileContext<'a> {
    /// A file's facts without a framework role, read from `source` as
    /// `language`: a document's Markdown view, a workflow, or a file only
    /// custom questions read.
    pub(super) fn plain(
        (owner, input): (usize, &'a crate::inventory::Input),
        (language, source): (&'static str, &'a str),
        args: &'a crate::options::CheckArgs,
        budget: Limits<'a>,
    ) -> Self {
        Self {
            owner,
            path: &input.result.path,
            language,
            source,
            source_hash: &input.result.source_hash,
            model: args.model(),
            budget,
            project: args.project.as_deref(),
            framework: None,
            changed: input.changed.as_ref(),
        }
    }

    /// Whether lines `start..=end` of this file are judged: the whole file
    /// is, or the change touched them.
    pub(super) fn judges(&self, start: usize, end: usize) -> bool {
        self.changed
            .is_none_or(|change| change.lines.touch(start, end))
    }

    /// Whether a unit of this file is judged: one of its locations here is.
    pub(super) fn judges_unit(&self, unit: &super::UnitPlan) -> bool {
        self.changed.is_none()
            || unit
                .locations
                .iter()
                .any(|l| l.path == self.path && self.judges(l.start_line, l.end_line))
    }

    /// Whether a rule about the whole file is asked: the whole file is
    /// judged, or the change adds one of `members` (the names the rule weighs
    /// and the lines they start on), which `known` lists for the file's base
    /// version.
    pub(super) fn adds<'n>(
        &self,
        members: impl Iterator<Item = (&'n str, usize)>,
        known: impl FnOnce(&Path, &str) -> std::collections::BTreeSet<String>,
    ) -> bool {
        self.changed
            .is_none_or(|change| change.adds(members, known))
    }

    pub(super) fn location(
        &self,
        start_line: usize,
        end_line: usize,
        symbol: Option<&str>,
    ) -> Location {
        Location {
            path: self.path.to_path_buf(),
            start_line,
            end_line,
            symbol: symbol.map(str::to_string),
        }
    }

    /// The file's path and language, for questions about how code reads:
    /// a framework role sent there moved split answers without informing them.
    pub(super) fn plain_state(&self) -> Value {
        let mut state = json!({"path": self.path, "language": self.language});
        if self.language == crate::analysis::bend::LANGUAGE {
            state["notation"] = json!(BEND_NOTATION);
        }
        state
    }

    /// The file's path, language and framework role, for questions about
    /// where values come from and go, and which values a reader must guess.
    pub(super) fn file_state(&self) -> Value {
        let mut state = self.plain_state();
        if let Some(framework) = &self.framework {
            state["framework"] = json!(framework);
        }
        state
    }

    pub(super) fn request(
        &self,
        stage: &str,
        state: Value,
        questions: Questions,
    ) -> (Value, Asked) {
        request(
            self.model,
            stage,
            &[(self.path, self.source_hash)],
            state,
            questions.reworded(self.language),
        )
    }
}

/// Freshness hashes and the stage stay in local metadata; only model, state
/// and questions are uploaded.
pub(super) fn request(
    model: &str,
    stage: &str,
    sources: &[(&Path, &str)],
    state: Value,
    questions: Questions,
) -> (Value, Asked) {
    let (mut questions, asked) = questions.finish();
    for fact in facts(&state) {
        for body in questions.values_mut() {
            point_to(body, fact);
        }
    }
    let sources: Vec<_> = sources
        .iter()
        .map(|(path, hash)| json!({"path": path, "source_hash": hash}))
        .collect();
    let request = json!({
        "model": model,
        "state": state,
        "questions": questions,
        "jevgate": {"stage": stage, "sources": sources},
    });
    (request, asked)
}

/// What a note adds when the state names the file's framework role.
const FRAMEWORK_NOTE: &str =
    "`file.framework` states who calls this file's code and where it runs.";

/// Bend 2's notation, sent beside its files' code: a model may know Bend 1,
/// a different language with the same extension, or no Bend at all.
const BEND_NOTATION: &str = "Bend 2 (bendlang/bend 2.0.x), not Bend 1: a pure, affine, dependently typed language. `def f(x: A, +y: B, -T: Type) -> R:` defines a function: `+y` may be used more than once, `-T` is erased (seen by types and proofs only), `~g` is a template argument inlined at compile time, and `@unsafe` skips the termination check. `match x:` with `case K{a, b}:` is the only branching and recursion replaces loops. `law name:` states a claim (`for x: A` is for every x, `exs y: B` asks for a witness, `where P` adds a hypothesis, `{a == b : T}` is an equality) that a `def` of the same name proves; a law no def fills declares a signature or a primitive. In proofs `{==}` is reflexivity, `%e : P` rewrites with the equality `e`, and `?name` or `?TODO` leaves a goal open. `(a + b : U32)` computes at type U32, `3n` is a Nat, `1n+p` matches a successor, `h <> t` builds a list and `++` joins strings. `do IO<T>:` sequences effects: `x : T <- m` binds a result and `return v` ends the block. A test file ends in `#|` lines, the output its run must print.";

const NOTATION_NOTE: &str = "`file.notation` explains the language's notation.";

/// The notes every question about `state` points to, for the facts it
/// holds beside the file's code, in the order they are added.
pub(super) fn facts(state: &Value) -> impl Iterator<Item = &'static str> {
    [
        (state["file"]["framework"].is_string(), FRAMEWORK_NOTE),
        (state["file"]["notation"].is_string(), NOTATION_NOTE),
    ]
    .into_iter()
    .filter_map(|(held, note)| held.then_some(note))
}

/// Point a question at a fact the state holds beside the file's code:
/// stated only in the state, a client component's role did not clear its
/// browser requests, since the questions never pointed at it.
pub(super) fn point_to(body: &mut Value, fact: &str) {
    let instructions = &mut body["instructions"];
    let note = match instructions["note"].as_str() {
        Some(note) => format!("{fact} {note}"),
        None => fact.to_string(),
    };
    instructions["note"] = Value::String(note);
}

/// Greedy packing in order: at most `limit` items and `PACK_BYTES` of state.
pub(super) fn pack<T>(items: Vec<T>, limit: usize, state: impl Fn(&T) -> &Value) -> Vec<Vec<T>> {
    let mut packs: Vec<Vec<T>> = Vec::new();
    let mut used = 0;
    for item in items {
        let size = serde_json::to_vec(state(&item)).map_or(0, |v| v.len());
        match packs.last_mut() {
            Some(pack) if pack.len() < limit && used + size <= PACK_BYTES => {
                used += size;
                pack.push(item);
            }
            _ => {
                used = size;
                packs.push(vec![item]);
            }
        }
    }
    packs
}

/// One in this many keys ends a run of items packed together. Every request
/// is billed about 280 input tokens beyond its size, so shorter runs cost
/// more on a full run: one in four bills 4% to 7% more first-pass input than
/// greedy packing, one in eight 2% to 3%. One in four re-asks 24% to 47%
/// fewer tokens per function removed, one in eight 12% to 31% and leaves
/// more files in a single run, so one in four is cheaper after 31 to 40
/// edits.
const RUN_ENDS: u8 = 4;

/// Packing within runs of consecutive items: a run ends after the last item
/// of a key (a definition's name, a section's heading) whose SHA-256 first
/// byte is a multiple of `RUN_ENDS`, so where runs end depends on keys
/// rather than positions, and an item added, removed or resized re-packs
/// only its own run. Packed greedily in file order, one such edit shifted
/// every later pack of the file and none of them hit the cache; packing
/// each key alone kept the others too, but doubled to quadrupled requests.
///
/// Only the items `kept` selects are packed, within runs that end where
/// they end for every item of the file: when `--base` judges only what a
/// change touched, the changed functions of one run share a pack, and a
/// later push that changes another function of the run adds it to that
/// pack, which is asked again whole.
pub(super) fn pack_runs<T>(
    items: Vec<T>,
    key: impl Fn(&T) -> &str,
    state: impl Fn(&T) -> &Value,
    kept: impl Fn(&T) -> bool,
) -> Vec<Vec<T>> {
    let ends: Vec<bool> = items
        .iter()
        .enumerate()
        .map(|(at, item)| {
            let own = key(item);
            let last = items.get(at + 1).is_none_or(|next| key(next) != own);
            last && Sha256::digest(own.as_bytes())[0] % RUN_ENDS == 0
        })
        .collect();
    let mut packs = Vec::new();
    let mut run = Vec::new();
    for (item, end) in items.into_iter().zip(ends) {
        if kept(&item) {
            run.push(item);
        }
        if end {
            packs.extend(pack(std::mem::take(&mut run), PACK_ITEMS, &state));
        }
    }
    packs.extend(pack(run, PACK_ITEMS, &state));
    packs
}

pub(super) fn identity(parts: &[&str]) -> String {
    crate::schema::hash(parts.join(crate::schema::HASH_SEPARATOR).as_bytes())
}

pub(super) fn compact(text: &str) -> String {
    text.split_whitespace().collect()
}

/// Unique ids for units that share a name in one file.
pub(super) fn unique_ids<'a>(prefix: &str, names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen = BTreeMap::<String, usize>::new();
    names
        .map(|name| {
            let count = seen.entry(name.to_string()).or_default();
            *count += 1;
            if *count == 1 {
                format!("{prefix}:{name}")
            } else {
                format!("{prefix}:{name}#{count}")
            }
        })
        .collect()
}
