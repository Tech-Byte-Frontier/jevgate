//! Custom questions' units and requests. A selected question that applies
//! to a file gets one unit per function, test, comment, documentation
//! section or changed hunk of it, or one for the whole file, as its `unit`
//! says. A unit whose evidence a built-in first-pass request of the file
//! already sends is asked there (`ride`); the others are asked in requests
//! of their own, packed as the built-in stage packs the same units.
mod hunks;
mod items;
mod ride;

use super::{
    Asked, Detail, FileContext, FilePlan, Planned, Presence, Questions, TEST_PACK_ITEMS, UnitPlan,
    pack, pack_runs, questions::EVIDENCE,
};
use crate::{
    analysis::{test_map::TestCase, units::Unit},
    custom::{Kind, Question},
    options::CheckArgs,
};
use items::Item;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    ops::Range,
    path::Path,
};

/// The stage of custom questions' own requests.
const STAGE: &str = "custom";

/// Units one question asks about in one run, at most; the rest of a broad
/// question on a large repository are counted as omitted. Beside the
/// built-in questions a unit adds its question's text, about 60 to 600
/// tokens, so the cap holds a question to about 1.2 million new input
/// tokens a run ($0.05); asked of whole files, about $0.2.
pub(crate) const MAX_UNITS: usize = 2_000;

/// What a code file offers custom questions.
pub(super) struct Code<'a> {
    pub units: &'a [Unit],
    /// Test code: functions and comments in it are not asked about.
    pub test_lines: &'a [Range<usize>],
    /// Whether its code outside `test_lines` is application code.
    pub application: bool,
    /// Its test cases, when tests are judged (`include_tests`).
    pub tests: Option<&'a [TestCase]>,
}

/// The kind of file a planner call is about, which decides the units its
/// questions can ask about.
enum Offered<'a> {
    Code(Code<'a>),
    /// Agent instructions or project documentation.
    Document,
    /// A file only `file` and `hunk` questions read: a source file without a
    /// parser (`source`), or a text file a question's `paths` name.
    Text {
        source: bool,
    },
}

/// A custom unit waiting for the request that asks it.
#[derive(Clone, Copy)]
struct Ask {
    /// The unit, in its file's plan.
    unit: usize,
    /// Its evidence, among the file's items of its kind.
    item: usize,
    question: &'static Question,
}

/// Plans the selected custom questions for every file of a run.
pub(super) struct Planner {
    questions: Vec<&'static Question>,
    /// The changes since `--base`, when a hunk question is selected.
    changes: Option<hunks::Changes>,
    /// Units each question asked so far in this run.
    asked: BTreeMap<&'static str, usize>,
}

impl Planner {
    pub(super) fn new(args: &CheckArgs, root: &Path) -> Self {
        let questions: Vec<_> = args.custom().collect();
        let hunk = questions.iter().any(|q| q.unit == Kind::Hunk);
        let changes = hunk.then(|| hunks::Changes::load(root, args)).flatten();
        Self {
            questions,
            changes,
            asked: BTreeMap::new(),
        }
    }

    /// Whether a `file` question, or a `hunk` question with `--base`, reads
    /// `path`, so a source file JevGate cannot parse is still asked about.
    pub(super) fn reads_text(&self, path: &Path) -> bool {
        self.questions.iter().any(|q| {
            let asked = q.unit == Kind::File || q.unit == Kind::Hunk && self.changes.is_some();
            asked && q.applies_to(path)
        })
    }

    /// A parsed code file's units, after its built-in planners planned
    /// `requests[since..]`.
    pub(super) fn code(
        &mut self,
        file: &FileContext<'_>,
        code: Code<'_>,
        out: &mut FilePlan,
        (requests, since): (&mut Vec<Planned>, usize),
    ) {
        self.plan(file, &Offered::Code(code), out, (requests, since));
    }

    /// A document's sections, after its built-in planners.
    pub(super) fn document(
        &mut self,
        file: &FileContext<'_>,
        out: &mut FilePlan,
        (requests, since): (&mut Vec<Planned>, usize),
    ) {
        self.plan(file, &Offered::Document, out, (requests, since));
    }

    /// A file no built-in rule reads: `source` when it is a source file
    /// without a parser, not one only a question's `paths` name.
    pub(super) fn text(
        &mut self,
        file: &FileContext<'_>,
        source: bool,
        out: &mut FilePlan,
        requests: &mut Vec<Planned>,
    ) {
        let since = requests.len();
        self.plan(file, &Offered::Text { source }, out, (requests, since));
    }

    fn plan(
        &mut self,
        file: &FileContext<'_>,
        offered: &Offered<'_>,
        out: &mut FilePlan,
        (requests, since): (&mut Vec<Planned>, usize),
    ) {
        let mut items = BTreeMap::<Kind, Vec<Item>>::new();
        let mut asks = BTreeMap::<Kind, Vec<Ask>>::new();
        for question in self.questions.clone() {
            if !applies(question, offered, file.path) {
                continue;
            }
            let found = match items.entry(question.unit) {
                Entry::Occupied(found) => found.into_mut(),
                Entry::Vacant(slot) => match self.items(file, offered, question.unit) {
                    Some(found) => slot.insert(found),
                    None => continue,
                },
            };
            let room = MAX_UNITS - self.asked.get(question.rule.as_str()).copied().unwrap_or(0);
            let taken = found.len().min(room);
            *out.rules.entry(&question.rule).or_default() += found.len() - taken;
            out.questions.push(question);
            *self.asked.entry(&question.rule).or_default() += taken;
            for (index, item) in found.iter().enumerate().take(taken) {
                out.units.push(unit(question, item));
                asks.entry(question.unit).or_default().push(Ask {
                    unit: out.units.len() - 1,
                    item: index,
                    question,
                });
            }
        }
        for (kind, asks) in asks {
            let found = &items[&kind];
            let rest = ride::ride(file, kind, found, asks, out, &mut requests[since..]);
            standalone(file, kind, found, rest, out, requests);
        }
    }

    /// The units of `kind` a file offers; none when the kind does not apply
    /// to it: functions and comments of application code, tests only when
    /// judged, sections of documents, hunks only with `--base`.
    fn items(
        &self,
        file: &FileContext<'_>,
        offered: &Offered<'_>,
        kind: Kind,
    ) -> Option<Vec<Item>> {
        match (kind, offered) {
            (Kind::Function, Offered::Code(code)) if code.application => {
                Some(items::functions(file, code.units, code.test_lines))
            }
            (Kind::Comment, Offered::Code(code)) if code.application => {
                Some(items::comments(file, code.units, code.test_lines))
            }
            (Kind::Test, Offered::Code(code)) => code.tests.map(|cases| items::tests(file, cases)),
            (Kind::Section, Offered::Document) => Some(items::sections(file)),
            (Kind::File, _) => Some(vec![items::whole(file)]),
            (Kind::Hunk, _) => {
                let hunks = self.changes.as_ref()?.hunks(file.path, file.source);
                Some(items::changed(file, &hunks))
            }
            _ => None,
        }
    }
}

/// Whether `question` is asked about a file at `path`: a question without
/// `paths` reads the files the built-in rules read for its kind; a `file`
/// or `hunk` question with `paths` also reads documents and other text.
fn applies(question: &Question, offered: &Offered<'_>, path: &Path) -> bool {
    question.applies_to(path)
        && match (offered, question.unit) {
            (Offered::Code(_), kind) => kind != Kind::Section,
            (Offered::Document, Kind::Section) => true,
            (Offered::Document, _) => question.names_files(),
            (Offered::Text { source }, Kind::File | Kind::Hunk) => {
                *source || question.names_files()
            }
            (Offered::Text { .. }, _) => false,
        }
}

/// The unit `question` asks about `item`.
fn unit(question: &'static Question, item: &Item) -> UnitPlan {
    UnitPlan {
        rule: &question.rule,
        id: format!("{}:{}", question.rule, item.id),
        name: item.name.clone(),
        presence: Presence::Judged,
        locations: vec![item.location.clone()],
        quote: item.quote.clone(),
        lines: item.lines,
        identity: item.identity.clone(),
        detail: Detail::Custom(question),
        recheck: None,
    }
}

/// The asks no built-in request took, in requests of their own: packed by
/// runs as the built-in stage packs the same units, tests and whole files
/// one per request. A pack too large is sent one unit at a time, and a unit
/// too large alone needs context.
fn standalone(
    file: &FileContext<'_>,
    kind: Kind,
    items: &[Item],
    asks: Vec<Ask>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let mut asked = BTreeMap::<usize, Vec<Ask>>::new();
    for ask in asks {
        asked.entry(ask.item).or_default().push(ask);
    }
    let entries: Vec<(&Item, Vec<Ask>)> = asked
        .into_iter()
        .map(|(item, asks)| (&items[item], asks))
        .collect();
    let packs = match kind {
        Kind::Test | Kind::File => pack(entries, TEST_PACK_ITEMS, |(item, _)| &item.state),
        _ => pack_runs(
            entries,
            |(item, _)| item.run.as_str(),
            |(item, _)| &item.state,
            |_| true,
        ),
    };
    for group in packs {
        let (request, asked) = build(file, kind, &group, out);
        if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
            continue;
        }
        for entry in group {
            let (request, asked) = build(file, kind, std::slice::from_ref(&entry), out);
            if file.budget.fits(&request) {
                requests.push(Planned {
                    owner: file.owner,
                    request,
                    asked,
                });
            } else {
                for ask in entry.1 {
                    out.units[ask.unit].presence = Presence::NeedsContext;
                }
            }
        }
    }
}

/// One request asking each entry's questions about its item.
fn build(
    file: &FileContext<'_>,
    kind: Kind,
    entries: &[(&Item, Vec<Ask>)],
    out: &FilePlan,
) -> (Value, Asked) {
    let mut questions = Questions::default();
    for (index, (_, asks)) in entries.iter().enumerate() {
        for ask in asks {
            questions.ask_custom(
                key(index, ask.question),
                body(ask.question, kind, index),
                &out.units[ask.unit].id,
                ask.question,
            );
        }
    }
    let mut state = json!({"file": file.plain_state()});
    match kind {
        Kind::File => state["file"]["source"] = json!(file.source),
        _ => {
            let listed: Vec<&Value> = entries.iter().map(|(item, _)| &item.state).collect();
            state[list(kind)] = json!(listed);
        }
    }
    file.request(STAGE, state, questions)
}

/// The language a file only custom questions read is stated in: its
/// extension's, a documentation format's, a server template as one, and
/// any other text as text.
pub(super) fn language(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if crate::docs::format::EXTENSIONS.contains(&extension.as_str()) {
        return crate::docs::format::Format::of(path).language();
    }
    if crate::components::server_template(path) {
        return "template";
    }
    match crate::file_kind::language(path) {
        "unknown" => "text",
        language => language,
    }
}

/// The state's list of units of `kind`, as the built-in requests name it.
fn list(kind: Kind) -> &'static str {
    match kind {
        Kind::Function => "functions",
        Kind::Test => "tests",
        Kind::Comment => "comments",
        Kind::Section => "sections",
        Kind::Hunk => "hunks",
        Kind::File => "file",
    }
}

/// The key of a question about the unit at `index` of its request.
fn key(index: usize, question: &Question) -> String {
    format!("custom_{index}_{}", question.id().replace('-', "_"))
}

/// A custom question as a Noul about the unit at `index`, named by its
/// literal state path; background and guidance ride as labeled keys.
fn body(question: &Question, kind: Kind, index: usize) -> Value {
    let mut instructions = json!({
        "question": format!("For {}: {}", subject(kind, index), question.question),
        "note": match kind {
            Kind::Hunk => format!("`hunks[{index}].diff` is a unified diff: lines starting with `+` were added, `-` removed, and a space are unchanged context around them. Judge what the change adds or alters. {EVIDENCE}"),
            _ => EVIDENCE.to_string(),
        },
    });
    if let Some(background) = &question.background {
        instructions["background"] = json!(background);
    }
    if let Some(guidance) = &question.guidance {
        instructions["guidance"] = json!(guidance);
    }
    json!({
        "type": "noul",
        "instructions": instructions,
        "criteria": {"true": "Yes.", "false": "No, or the question does not apply to it."},
    })
}

/// The unit at `index` of a request, named by its literal state paths.
fn subject(kind: Kind, index: usize) -> String {
    match kind {
        Kind::Function => format!("the function in `functions[{index}].source`"),
        Kind::Test => format!("the test in `tests[{index}].source`"),
        Kind::Comment => format!(
            "the comment in `comments[{index}].text`, about the code in `comments[{index}].code`"
        ),
        Kind::Section => format!(
            "the section in `sections[{index}].text`, under the heading in `sections[{index}].heading`"
        ),
        Kind::Hunk => format!("the change in `hunks[{index}].diff`"),
        Kind::File => "the file in `file.source`".to_string(),
    }
}
