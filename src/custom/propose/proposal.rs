//! A proposed question: a line Jev calls a rule, quoted verbatim with its
//! file and line, its section heading as background and the text that
//! introduces it as guidance, on the unit Jev chose, as a note.
use super::{
    ask::{Answered, Asked},
    files::File,
    lines::Line,
    saved::Saved,
};
use crate::custom::{Kind, QUESTION_CHARS, TEXT_CHARS};
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeSet;

/// The probability of a rule at or above which a line is proposed: the
/// threshold of review findings. On the instruction files of six projects
/// never used to write the questions, 197 of the 271 lines proposed were
/// rules a labeler would keep as questions, 14 were not and 60 were
/// debatable; of the 50 lines between 0.65 and 0.80, 6 were.
pub const THRESHOLD: f64 = crate::policy::REVIEW_PROBABILITY;

/// The probability that a formatter, linter, compiler or measuring script
/// already checks a rule, at or above which it is not proposed: a question
/// would repeat, at a price, a check the project runs anyway. On 33
/// projects' instruction files it left out 15 of the 25 proposals labeled
/// wrong (line and complexity budgets, line length) and none of the 97
/// labeled right; on the six fresh projects, none of 271.
pub const TOOL_CHECKED: f64 = crate::policy::REVIEW_PROBABILITY;

/// Whether the first pass calls a line a rule.
pub fn rule(answered: &Answered) -> bool {
    crate::policy::probability_at_least(answered.convention, THRESHOLD)
}

/// Whether a line is proposed: Jev calls it a rule, and not one a tool
/// already checks. A rule whose second answer is missing is not proposed.
pub fn proposed(answered: &Answered) -> bool {
    rule(answered)
        && answered
            .tool_checked()
            .is_some_and(|p| !crate::policy::probability_at_least(p, TOOL_CHECKED))
}

/// The comment that marks a proposal with a hash of its rule, so a later run
/// knows the rule was proposed or accepted, whatever a person changed.
pub const MARKER: &str = "# jevgate-proposal:";

/// Hex digits of a rule's hash in its marker.
const MARKER_CHARS: usize = 12;

/// Characters of an id taken from a rule's first words.
const ID_CHARS: usize = 40;

/// Characters of a heading quoted in the background.
const HEADING_CHARS: usize = 200;

/// Where a proposal stands against what is already saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Not proposed before.
    New,
    /// In `.jevgate/proposals/` from an earlier run; kept as it is.
    Kept,
    /// Already a question in `.jevgate/questions/` or `jevgate.toml`.
    Accepted,
    /// The same rule as an earlier line of this run, such as a copy in CLAUDE.md.
    Repeated,
}

/// One candidate line with its answers and, when Jev calls it a rule, its
/// proposal.
pub struct Candidate<'a> {
    pub file: &'a File,
    pub line: &'a Line,
    pub answered: Option<Answered>,
    pub proposal: Option<Proposal>,
}

/// A custom question proposed from one line.
#[derive(Debug)]
pub struct Proposal {
    pub id: String,
    pub status: Status,
    /// A hash of the rule's text.
    pub marker: String,
    pub question: String,
    pub background: String,
    pub guidance: Option<String>,
    pub unit: Kind,
    pub paths: Vec<String>,
    /// The file and line it quotes: `AGENTS.md:12`.
    pub citation: String,
    pub convention: f64,
    pub unit_probability: f64,
}

/// Every candidate of `files` with its answers and proposal, in file order.
pub fn decide<'a>(files: &'a [File], asked: &Asked, saved: &mut Saved) -> Vec<Candidate<'a>> {
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for (at_file, file) in files.iter().enumerate() {
        for (at_line, line) in file.lines.iter().enumerate() {
            let answered = asked.answered(at_file, at_line);
            let proposal = answered.as_ref().filter(|a| proposed(a)).map(|a| {
                let mut proposal = Proposal::new(file, line, a);
                proposal.status = if !seen.insert(proposal.marker.clone()) {
                    Status::Repeated
                } else {
                    saved.place(&mut proposal.id, &proposal.marker)
                };
                proposal
            });
            candidates.push(Candidate {
                file,
                line,
                answered,
                proposal,
            });
        }
    }
    candidates
}

impl Proposal {
    fn new(file: &File, line: &Line, answered: &Answered) -> Self {
        let path = slashed(&file.path);
        let (unit, unit_probability) = answered.unit();
        let citation = format!("{path}:{}", line.start_line);
        let (question, quoted) = question(unit, &line.text, (&path, line.start_line));
        Self {
            id: slug(&line.text).unwrap_or_else(|| fallback_id(&file.path, line.start_line)),
            status: Status::New,
            marker: marker(&line.text),
            question,
            background: background(&path, &line.heading),
            guidance: guidance(&path, line, quoted),
            unit,
            paths: file.scope.clone(),
            citation,
            convention: answered.convention,
            unit_probability,
        }
    }

    /// The question file `jevgate rules accept` moves into place.
    pub fn file(&self) -> Result<String> {
        let accept = format!("Edit it, then accept it: jevgate rules accept {}", self.id);
        Ok(format!(
            "{}{}",
            self.header(&accept),
            toml::to_string(&self.fields(None))?
        ))
    }

    /// A `[[question]]` table for `jevgate.toml`.
    pub fn table(&self) -> Result<String> {
        let tables = Tables {
            question: [self.fields(Some(&self.id))],
        };
        Ok(format!(
            "{}{}",
            self.header("Edit it in jevgate.toml, where it is asked as it is."),
            toml::to_string(&tables)?
        ))
    }

    /// Where it comes from, what Jev answered, how to accept it, and its
    /// marker, as TOML comments.
    fn header(&self, accept: &str) -> String {
        [
            format!("Proposed by `jevgate rules propose` from {}.", self.citation),
            format!(
                "Jev: a rule to check ({:.2}), on each {} ({:.2}).",
                self.convention,
                self.unit.noun(),
                self.unit_probability
            ),
            accept.to_string(),
            "It starts as a note, which never fails the gate. A rule quoted alone can answer close to".into(),
            "the threshold: before raising level to \"review\", add guidance (what breaks the rule and".into(),
            format!("what only looks like it) and a [[failing]] and a [[passing]] example, and run `jevgate rules test --rule custom/{}`.", self.id),
        ]
        .iter()
        .map(|line| format!("# {line}\n"))
        .chain([format!("{MARKER} {}\n", self.marker)])
        .collect()
    }

    fn fields(&self, id: Option<&str>) -> Fields<'_> {
        Fields {
            id: id.map(str::to_string),
            question: &self.question,
            background: &self.background,
            guidance: self.guidance.as_deref(),
            unit: self.unit,
            paths: &self.paths,
            level: "note",
        }
    }

    /// The question's fields, for JSON output.
    pub fn describe(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self.fields(Some(&self.id))).unwrap_or_default();
        value["status"] = serde_json::json!(self.status);
        value["from"] = serde_json::json!(self.citation);
        value
    }
}

/// A proposal's keys, in the order a question file lists them.
#[derive(Serialize)]
struct Fields<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    question: &'a str,
    background: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    guidance: Option<&'a str>,
    unit: Kind,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    paths: &'a [String],
    level: &'static str,
}

/// One `[[question]]` table.
#[derive(Serialize)]
struct Tables<'a> {
    question: [Fields<'a>; 1],
}

/// What a finding calls the unit in the question's words.
fn noun(unit: Kind) -> &'static str {
    match unit {
        Kind::Hunk => "change",
        other => other.noun(),
    }
}

/// `Does this function break the project rule "…" (AGENTS.md:12)?`, and
/// whether the rule is quoted whole: a rule too long for the question is
/// quoted up to its last sentence that fits. A path too long to cite leaves
/// its file name.
fn question(unit: Kind, text: &str, (path, line): (&str, usize)) -> (String, bool) {
    let frame = |quote: &str, citation: &str| {
        format!(
            "Does this {} break the project rule \"{quote}\" ({citation})?",
            noun(unit)
        )
    };
    let mut citation = format!("{path}:{line}");
    if frame("…", &citation).chars().count() > QUESTION_CHARS / 2 {
        let name = path.rsplit('/').next().unwrap_or(path);
        citation = format!("{name}:{line}");
    }
    let room = QUESTION_CHARS.saturating_sub(frame("", &citation).chars().count());
    let quote = shortened(text, room);
    let whole = quote == text;
    (frame(&quote, &citation), whole)
}

/// `text` when it has at most `room` characters; else up to its last
/// sentence end in the second half of that room, or its last whole word,
/// and `…`.
fn shortened(text: &str, room: usize) -> String {
    if text.chars().count() <= room {
        return text.to_string();
    }
    let head: String = text.chars().take(room.saturating_sub(1)).collect();
    let bytes = head.as_bytes();
    let sentence = (bytes.len() / 2..bytes.len()).rev().find(|&at| {
        at > 0 && matches!(bytes[at - 1], b'.' | b';' | b'!' | b'?') && bytes[at] == b' '
    });
    let cut = sentence.or_else(|| head.rfind(' ')).unwrap_or(head.len());
    format!("{}…", head[..cut].trim_end())
}

/// The section the rule is in, which the brief sends as background.
fn background(path: &str, heading: &str) -> String {
    if heading.is_empty() {
        format!("The rule is from {path}.")
    } else {
        let heading = shortened(heading, HEADING_CHARS);
        format!("The rule is from {path}, section \"{heading}\".")
    }
}

/// What belongs to the rule beyond its quote: the text that introduces it,
/// and the rule whole when the question quotes only its start. Its sibling
/// lines are other rules, and are not sent.
fn guidance(path: &str, line: &Line, quoted_whole: bool) -> Option<String> {
    let whole = (!quoted_whole).then(|| format!("The whole rule: \"{}\"", line.text));
    let used = whole.as_ref().map_or(0, |w| w.chars().count());
    let lead_in = line.lead_in.as_ref().map(|lead_in| {
        let frame = format!("In {path}, the rule comes under \"\".");
        let room = TEXT_CHARS.saturating_sub(used + frame.chars().count() + 1);
        format!(
            "In {path}, the rule comes under \"{}\".",
            shortened(lead_in, room)
        )
    });
    let parts: Vec<String> = lead_in.into_iter().chain(whole).collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// A hash of the rule's text, for its marker.
fn marker(text: &str) -> String {
    let mut hash = crate::schema::hash(text.as_bytes());
    hash.truncate(MARKER_CHARS);
    hash
}

/// An id from the rule's first words: lowercase ASCII letters and digits
/// joined by hyphens, up to [`ID_CHARS`], starting with a letter; from its
/// first sentence when that has two such words, so ky's "Prefer `undefined`
/// for absent values. Do not add special handling for `null`." is
/// `prefer-undefined-for-absent-values`, not `…-values-do`. None when the
/// rule has no such word, as in a rule written in Chinese.
pub fn slug(text: &str) -> Option<String> {
    let first = hyphenated(first_sentence(text));
    if first.contains('-') {
        return Some(first);
    }
    let whole = hyphenated(text);
    (!whole.is_empty()).then_some(whole)
}

/// The ASCII words of `text` from its first that starts with a letter,
/// lowercase, joined by hyphens while they fit [`ID_CHARS`].
fn hyphenated(text: &str) -> String {
    let ascii: String = text.chars().filter(char::is_ascii).collect();
    let mut id = String::new();
    let words = ascii
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .skip_while(|word| !word.starts_with(|c: char| c.is_ascii_alphabetic()));
    for word in words {
        if id.len() + usize::from(!id.is_empty()) + word.len() > ID_CHARS {
            break;
        }
        if !id.is_empty() {
            id.push('-');
        }
        id.push_str(&word.to_ascii_lowercase());
    }
    id
}

/// `text` up to the end of its first sentence: a `.`, `!` or `?` followed by
/// a space and a capital letter or code, so "e.g. this" ends none.
fn first_sentence(text: &str) -> &str {
    let ends = text.match_indices(['.', '!', '?']).map(|(at, _)| at);
    ends.into_iter()
        .find(|&at| {
            text[at + 1..]
                .strip_prefix(' ')
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_uppercase() || c == '`'))
        })
        .map_or(text, |at| &text[..at])
}

/// An id from the file's name and the rule's line: `agents-12`.
fn fallback_id(path: &std::path::Path, line: usize) -> String {
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = slug(&name).unwrap_or_else(|| "rule".into());
    format!("{stem}-{line}")
}

/// A relative path with `/` between its parts, as questions cite it, of
/// [`printable`](crate::custom::characters::printable) characters: a directory name can
/// hold a line break, which would end a proposal's comment and start a key.
pub fn slashed(path: &std::path::Path) -> String {
    path.iter()
        .map(|part| {
            let part = part.to_string_lossy();
            part.chars()
                .filter(|c| crate::custom::characters::printable(*c))
                .collect()
        })
        .collect::<Vec<String>>()
        .join("/")
}
