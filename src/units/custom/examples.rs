//! A custom question's examples, asked as a check asks the same units: the
//! units of the question's kind in a file at the example's path holding its
//! text, in requests of stage `custom` packed as a check packs units no
//! built-in request sends. `jevgate check --rule custom/<id>` of a file with
//! that path and text sends the same requests, so the two share answers.
use super::{Ask, hunks, items, items::Item, standalone, unit};
use crate::{
    custom::{Kind, Question},
    schema::Answer,
    token_budget::Limits,
    units::{FileContext, FilePlan, Planned, Presence, outcome, wording::custom_subject},
};
use anyhow::{Result, ensure};
use serde_json::json;
use std::path::Path;

/// One unit of an example: the id its answers come back under, and how a
/// finding would name it.
pub(crate) struct ExampleUnit {
    pub id: String,
    pub subject: String,
}

/// What asking an example takes.
pub(crate) struct ExamplePlan {
    pub units: Vec<ExampleUnit>,
    pub requests: Vec<Planned>,
}

/// The example's file, as a check would read it.
pub(crate) struct ExampleFile<'a> {
    /// Which of the run's examples it is; its requests are planned for it.
    pub owner: usize,
    /// The file it stands for.
    pub path: &'a Path,
    pub text: &'a str,
}

/// The units `question` asks about in `example` and the requests that ask
/// them. An example without such a unit, in a language a `function`,
/// `comment` or `test` question cannot parse, or too large to ask in one
/// request is an error: it cannot say whether the question separates
/// anything.
pub(crate) fn plan(
    question: &'static Question,
    example: &ExampleFile<'_>,
    (model, budget): (&str, Limits<'_>),
) -> Result<ExamplePlan> {
    let (language, source) = read_as(question.unit, example);
    let hash = crate::schema::hash(example.text.as_bytes());
    let file = FileContext {
        owner: example.owner,
        path: example.path,
        language,
        source: &source,
        source_hash: &hash,
        model,
        budget,
        framework: None,
        project: None,
        // An example is judged whole, as a file a check selects without --base.
        changed: None,
    };
    let found = items_of(question.unit, &file)?;
    ensure!(!found.is_empty(), "{}", missing(question.unit));
    let mut out = FilePlan {
        path: example.path.to_path_buf(),
        questions: vec![question],
        ..Default::default()
    };
    let mut asks = Vec::new();
    for (index, item) in found.iter().enumerate() {
        out.units.push(unit(question, item));
        asks.push(Ask {
            unit: out.units.len() - 1,
            item: index,
            question,
        });
    }
    let mut requests = Vec::new();
    standalone(&file, question.unit, &found, asks, &mut out, &mut requests);
    ensure!(
        out.units
            .iter()
            .all(|u| u.presence != Presence::NeedsContext),
        "it is too large to ask about in one request"
    );
    // No file on disk stands behind an example, so nothing is rechecked
    // for freshness before it is sent.
    for planned in &mut requests {
        planned.request["jevgate"]["sources"] = json!([]);
    }
    let units = out
        .units
        .into_iter()
        .map(|u| ExampleUnit {
            subject: custom_subject(question.unit, &u.name),
            id: u.id,
        })
        .collect();
    Ok(ExamplePlan { units, requests })
}

/// The probability of yes in `answer` and whether a check reports it as a
/// finding of `question`; none for an answer that is not a Noul.
pub(crate) fn verdict(question: &Question, answer: &Answer) -> Option<(f64, bool)> {
    let Answer::Noul { noul } = answer else {
        return None;
    };
    let found = matches!(
        outcome::custom_outcome(question, answer),
        outcome::Outcome::Review(_) | outcome::Outcome::Consider(_) | outcome::Outcome::Note(_)
    );
    Some((*noul, found))
}

/// The language a check states for the example's file and the text it
/// reads: a document's sections as Markdown; code in its language; any
/// other file, and a diff, as a file only custom questions read.
fn read_as<'a>(kind: Kind, example: &ExampleFile<'a>) -> (&'static str, std::borrow::Cow<'a, str>) {
    let path = example.path;
    let language = match kind {
        Kind::Section => crate::docs::format::Format::of(path).language(),
        _ if crate::syntax::supported(path) => crate::file_kind::read_language(path, example.text),
        _ => super::language(path),
    };
    let source = match kind {
        Kind::Section => crate::docs::format::view(path, example.text),
        _ => std::borrow::Cow::Borrowed(example.text),
    };
    (language, source)
}

/// The units of `kind` in the example's file.
fn items_of(kind: Kind, file: &FileContext<'_>) -> Result<Vec<Item>> {
    Ok(match kind {
        Kind::Section => items::sections(file),
        Kind::File => vec![items::whole(file)],
        Kind::Hunk => items::changed(file, &hunks::example(file.source), &[]),
        Kind::Function | Kind::Comment | Kind::Test => {
            let parsed = crate::analysis::units::parse(file.path, file.source)?;
            ensure!(
                parsed.parsed,
                "JevGate has no {} parser, and a {} question needs one to find its units",
                file.language,
                kind.noun()
            );
            let found = match kind {
                Kind::Function => items::functions(file, &parsed.units, &[]),
                Kind::Comment => items::comments(file, &parsed, &[]),
                _ => items::tests(
                    file,
                    &crate::analysis::test_map::cases(file.path, file.source)?,
                ),
            };
            // A check skips a file whose syntax errors leave nothing to
            // judge, with that reason; "it holds no function" would send the
            // author looking for one.
            ensure!(
                !(found.is_empty() && parsed.partial()),
                "the {} parser could not read it",
                file.language
            );
            found
        }
    })
}

/// Why an example holds nothing to ask about.
fn missing(kind: Kind) -> String {
    match kind {
        Kind::Hunk => "it holds no changed line: start added lines with `+` and removed ones with `-`".into(),
        Kind::Test => "it holds no test case; give it a test file's path where its language needs one, such as tests/test_api.py".into(),
        Kind::Section => "it holds no heading section with text".into(),
        kind => format!("it holds no {}", kind.noun()),
    }
}
