//! Staleness and duplication across documents. Code selects the candidates:
//! sections naming paths or scripts the repository lacks, and pairs of
//! sections sharing much of their wording. A document whose release Git has
//! tagged, or whose named paths Git shows deleted, is first asked from its
//! headings whether it is a plan; a plan with such facts is finished, and one
//! finding covers it. The section and pair checks follow only for documents
//! that are not finished plans.
use super::{
    Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan, compact, identity,
    request, unique_ids,
};
use crate::{
    catalog::{DOC_DUPLICATION, DOC_STALENESS},
    docs::{
        Repository,
        markdown::{self, Section},
        overlap,
        references::{self, Fate, Missing},
    },
    inventory::Input,
    schema::Pass,
};
use serde_json::{Value, json};
use std::path::Path;

/// The facts shown of a document's deleted paths.
const LISTED_PATHS: usize = 5;

/// Two sections as (document, section) indexes.
type SectionPair = ((usize, usize), (usize, usize));

struct Doc<'a> {
    owner: usize,
    input: &'a Input,
    sections: Vec<Section>,
    missing: Vec<Vec<Missing>>,
    facts: Vec<String>,
    /// What the document is about, sent beside its sections: two sections
    /// headed `Installing` differ when one installs Gunicorn and the other
    /// Waitress.
    title: String,
    /// The directory of the package a project document belongs to.
    package: Option<std::path::PathBuf>,
}

/// Candidates across every selected document.
pub(super) struct Shared<'a> {
    docs: Vec<Doc<'a>>,
    pairs: Vec<SectionPair>,
    omitted: usize,
    staleness: bool,
    duplication: bool,
}

impl<'a> Shared<'a> {
    pub(super) fn new(
        inputs: &'a [Input],
        owners: &[usize],
        staleness: bool,
        duplication: bool,
    ) -> Self {
        let mut docs = Vec::new();
        for &owner in owners {
            let input = &inputs[owner];
            let Some(repository) = &input.repository else {
                continue;
            };
            let source = crate::docs::format::view(
                &input.result.path,
                input.source.as_deref().unwrap_or(""),
            );
            let package = (input.result.role == crate::inventory::DOCS)
                .then(|| crate::packages::package(&repository.root, &input.result.path))
                .flatten()
                .map(|p| p.dir);
            let parsed = markdown::parse(&source);
            let title = title(&source, &parsed);
            let mut sections = parsed.sections;
            // Markup dropped from other formats leaves runs of blank lines.
            if crate::docs::format::Format::of(&input.result.path)
                != crate::docs::format::Format::Markdown
            {
                for section in &mut sections {
                    section.text = crate::docs::format::collapse_blank_lines(&section.text);
                }
            }
            let missing: Vec<Vec<Missing>> = sections
                .iter()
                .map(|s| {
                    if staleness {
                        references::missing(
                            &repository.root,
                            &input.result.path,
                            &s.text,
                            &repository.history,
                            &repository.scripts,
                        )
                    } else {
                        Vec::new()
                    }
                })
                .collect();
            let facts = if staleness {
                facts(repository, &input.result.path, &title, &missing)
            } else {
                Vec::new()
            };
            docs.push(Doc {
                owner,
                input,
                sections,
                missing,
                facts,
                title,
                package,
            });
        }
        let (pairs, omitted) = if duplication {
            section_pairs(&docs)
        } else {
            (Vec::new(), 0)
        };
        Self {
            docs,
            pairs,
            omitted,
            staleness,
            duplication,
        }
    }

    /// Plan the staleness and duplication units of one document.
    pub(super) fn plan(
        &self,
        file: &FileContext<'_>,
        out: &mut FilePlan,
        requests: &mut Vec<Planned>,
    ) {
        let Some(d) = self.docs.iter().position(|doc| doc.owner == file.owner) else {
            return;
        };
        if self.staleness {
            out.rules.insert(DOC_STALENESS, 0);
            self.plan_staleness(d, file, out, requests);
        }
        if self.duplication {
            out.rules
                .insert(DOC_DUPLICATION, if d == 0 { self.omitted } else { 0 });
            self.plan_pairs(d, file, out);
        }
    }

    fn plan_staleness(
        &self,
        d: usize,
        file: &FileContext<'_>,
        out: &mut FilePlan,
        requests: &mut Vec<Planned>,
    ) {
        let doc = &self.docs[d];
        if !doc.facts.is_empty() {
            plan_unit(doc, file, out, requests);
        }
        let stale: Vec<usize> = (0..doc.sections.len())
            .filter(|&s| !doc.missing[s].is_empty())
            .collect();
        let ids = unique_ids("stale", stale.iter().map(|&s| heading(&doc.sections[s])));
        for (&s, id) in stale.iter().zip(ids) {
            out.units.push(stale_section(
                file,
                &doc.title,
                &doc.sections[s],
                &doc.missing[s],
                id,
            ));
        }
    }

    fn plan_pairs(&self, d: usize, file: &FileContext<'_>, out: &mut FilePlan) {
        // Each pair this document owns, as its own section and the other.
        let owned: Vec<SectionPair> = self
            .pairs
            .iter()
            .filter(|(a, b)| self.owner_of(*a, *b) == d)
            .map(|&(a, b)| if a.0 == d { (a, b) } else { (b, a) })
            .collect();
        let names: Vec<String> = owned
            .iter()
            .map(|(mine, other)| {
                let other_doc = &self.docs[other.0];
                format!(
                    "{}~{}:{}",
                    heading(&self.docs[d].sections[mine.1]),
                    other_doc.input.result.path.display(),
                    heading(&other_doc.sections[other.1])
                )
            })
            .collect();
        let ids = unique_ids("pair", names.iter().map(String::as_str));
        for ((mine, other), id) in owned.into_iter().zip(ids) {
            out.units.push(self.pair_unit(file, mine, other, id));
        }
    }

    /// The unit of one section pair: its check and the settle asked when the
    /// check stays undecided, both about the two sections side by side.
    fn pair_unit(
        &self,
        file: &FileContext<'_>,
        mine: (usize, usize),
        other: (usize, usize),
        id: String,
    ) -> UnitPlan {
        let doc = &self.docs[mine.0];
        let section = &doc.sections[mine.1];
        let other_doc = &self.docs[other.0];
        let other_section = &other_doc.sections[other.1];
        let other_path = &other_doc.input.result.path;
        let state = json!({
            "section_a": {"path": file.path, "document": doc.title, "heading": section.heading, "text": section.text},
            "section_b": {"path": other_path, "document": other_doc.title, "heading": other_section.heading, "text": other_section.text},
        });
        let sources = [
            (file.path, file.source_hash),
            (other_path, other_doc.input.result.source_hash.as_str()),
        ];
        let mut relation = Questions::default();
        relation.ask(
            "relation".into(),
            super::questions::pair_relation(),
            &id,
            DOC_DUPLICATION,
            "relation",
            Pass::Settle,
        );
        let settle = request(file.model, "doc-checks", &sources, state.clone(), relation);
        let questions = pair_questions(&id);
        let (request, asked) = request(file.model, "doc-checks", &sources, state, questions);
        let fits = file.budget.fits(&request);
        let other = crate::schema::Location {
            path: other_path.clone(),
            start_line: other_section.start_line,
            end_line: other_section.end_line,
            symbol: Some(heading(other_section).to_string()),
        };
        UnitPlan {
            rule: DOC_DUPLICATION,
            id,
            name: heading(section).to_string(),
            presence: if fits {
                Presence::Judged
            } else {
                Presence::NeedsContext
            },
            locations: vec![
                file.location(section.start_line, section.end_line, Some(heading(section))),
                other.clone(),
            ],
            quote: None,
            lines: section.end_line + 1 - section.start_line,
            identity: identity(&[&compact(&section.text), &compact(&other_section.text)]),
            detail: Detail::DocPair {
                other,
                check: fits.then(|| (request, asked).into()),
                settle: fits.then(|| settle.into()),
            },
            recheck: None,
        }
    }

    /// A pair belongs to its side in an agent instruction file, which every
    /// session pays for, else to its first side.
    fn owner_of(&self, a: (usize, usize), b: (usize, usize)) -> usize {
        let agent = |d: usize| self.docs[d].input.result.role == crate::inventory::INSTRUCTIONS;
        if agent(b.0) && !agent(a.0) { b.0 } else { a.0 }
    }
}

/// The questions asked of a section pair: whether each section states
/// everything the other states, whether they disagree, whether they are
/// about one subject, and whether one translates the other.
fn pair_questions(id: &str) -> Questions {
    let mut questions = Questions::default();
    for (key, body) in [
        (
            "a_covers",
            super::questions::pair_covers("section_a", "section_b"),
        ),
        (
            "b_covers",
            super::questions::pair_covers("section_b", "section_a"),
        ),
        ("conflict", super::questions::pair_conflict()),
        ("subject", super::questions::pair_subject()),
        ("translation", super::questions::pair_translation()),
    ] {
        questions.ask(key.into(), body, id, DOC_DUPLICATION, key, Pass::Trace);
    }
    questions
}

/// Sections of different documents that share much of their wording, as
/// (document, section) pairs, with the number of candidates over the caps.
/// Project documents of separate packages are not paired: each package's
/// README is read on its own, such as on a registry page, so repeating the
/// setup it shares with its siblings is how it stays complete.
fn section_pairs(docs: &[Doc<'_>]) -> (Vec<SectionPair>, usize) {
    let mut keys = Vec::new();
    let mut texts = Vec::new();
    for (d, doc) in docs.iter().enumerate() {
        for (s, section) in doc.sections.iter().enumerate() {
            keys.push((d, s));
            texts.push(overlap::Text {
                file: d,
                text: &section.text,
            });
        }
    }
    let comparable = |a: usize, b: usize| match (&docs[a].package, &docs[b].package) {
        (Some(x), Some(y)) => x.starts_with(y) || y.starts_with(x),
        _ => true,
    };
    let (pairs, omitted) = overlap::pairs(&texts, &comparable);
    (
        pairs.iter().map(|(a, b, _)| (keys[*a], keys[*b])).collect(),
        omitted,
    )
}

/// The question whether a document with Git facts of finished work is a plan.
fn plan_unit(
    doc: &Doc<'_>,
    file: &FileContext<'_>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let mut questions = Questions::default();
    questions.ask(
        "plan".into(),
        super::questions::document_plan(),
        PLAN,
        DOC_STALENESS,
        "plan",
        Pass::First,
    );
    let state =
        json!({"file": {"path": file.path}, "outline": super::documents::outline_of(file.source)});
    let (request, asked) = file.request("docs", state, questions);
    let fits = file.budget.fits(&request);
    let lines = file.source.lines().count().max(1);
    out.units.push(UnitPlan {
        rule: DOC_STALENESS,
        id: PLAN.into(),
        name: file.path.display().to_string(),
        presence: if fits {
            Presence::Judged
        } else {
            Presence::NeedsContext
        },
        locations: vec![file.location(1, lines, None)],
        quote: None,
        lines,
        identity: identity(&[PLAN, &doc.facts.join("|")]),
        detail: Detail::Plan {
            facts: doc.facts.clone(),
        },
        recheck: None,
    });
    if fits {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
    }
}

/// A section that names paths or scripts the repository lacks, with its
/// check whether it relies on them, asked only when its document is not a
/// finished plan.
fn stale_section(
    file: &FileContext<'_>,
    title: &str,
    section: &Section,
    missing: &[Missing],
    id: String,
) -> UnitPlan {
    let mut questions = Questions::default();
    questions.ask(
        "relies".into(),
        super::questions::section_relies(),
        &id,
        DOC_STALENESS,
        "relies",
        Pass::Trace,
    );
    let listed: Vec<Value> = missing
        .iter()
        .map(|m| {
            let kind = if m.fate == Fate::NoScript {
                "script"
            } else {
                "path"
            };
            json!({kind: m.name, "status": m.status()})
        })
        .collect();
    let state = json!({
        "file": {"path": file.path, "title": title},
        "section": {"heading": section.heading, "text": section.text},
        "missing": listed,
    });
    let mut role = Questions::default();
    role.ask(
        "role".into(),
        super::questions::missing_role(),
        &id,
        DOC_STALENESS,
        "role",
        Pass::Settle,
    );
    let settle = file.request("doc-checks", state.clone(), role);
    let (request, asked) = file.request("doc-checks", state, questions);
    let fits = file.budget.fits(&request);
    UnitPlan {
        rule: DOC_STALENESS,
        id,
        name: heading(section).to_string(),
        presence: if fits {
            Presence::Judged
        } else {
            Presence::NeedsContext
        },
        locations: vec![file.location(
            section.start_line,
            section.end_line,
            Some(heading(section)),
        )],
        quote: None,
        lines: section.end_line + 1 - section.start_line,
        identity: identity(&[&section.heading, &compact(&section.text)]),
        detail: Detail::Stale {
            missing: missing.iter().map(describe).collect(),
            check: fits.then(|| (request, asked).into()),
            settle: fits.then(|| settle.into()),
        },
        recheck: None,
    }
}

/// The unit id of a document's finished-plan question.
pub(super) const PLAN: &str = "plan";

fn heading(section: &Section) -> &str {
    if section.heading.is_empty() {
        super::instructions::PREAMBLE
    } else {
        &section.heading
    }
}

fn describe(missing: &Missing) -> String {
    match &missing.fate {
        Fate::Deleted => format!("`{}`, which was deleted", missing.name),
        Fate::Renamed(to) => format!(
            "`{}`, which was renamed to `{}`",
            missing.name,
            to.display()
        ),
        Fate::Absent => format!("`{}`, which is not in the repository", missing.name),
        Fate::Nearby(path) => format!(
            "`{}`, which is not in the repository, though `{}` is",
            missing.name,
            path.display()
        ),
        Fate::NoScript => format!(
            "`{}`, which no manifest declares as a script or dependency",
            missing.name
        ),
    }
}

/// What Git shows that the document's work is done: a release tag for a
/// version it names in its path or title, and paths it names that were
/// deleted or renamed since.
fn facts(
    repository: &Repository,
    path: &Path,
    title: &str,
    missing: &[Vec<Missing>],
) -> Vec<String> {
    let mut facts = release_tags(repository, path, title);
    facts.extend(removed_paths(missing));
    facts
}

/// A release tag for each version the document's path or title names.
fn release_tags(repository: &Repository, path: &Path, title: &str) -> Vec<String> {
    let mut facts = Vec::new();
    for version in versions(&format!("{} {title}", path.display())) {
        let tagged = [format!("v{version}"), version.clone()]
            .iter()
            .any(|t| repository.history.tags.contains(t));
        if tagged && !facts.iter().any(|f: &String| f.contains(&version)) {
            facts.push(format!("the repository has a release tag v{version}"));
        }
    }
    facts
}

/// How many paths the document names were deleted or renamed, with a few of them.
fn removed_paths(missing: &[Vec<Missing>]) -> Option<String> {
    let mut removed: Vec<&str> = Vec::new();
    for m in missing.iter().flatten() {
        if matches!(m.fate, Fate::Deleted | Fate::Renamed(_)) && !removed.contains(&m.name.as_str())
        {
            removed.push(&m.name);
        }
    }
    if removed.is_empty() {
        return None;
    }
    let shown: Vec<String> = removed
        .iter()
        .take(LISTED_PATHS)
        .map(|n| format!("`{n}`"))
        .collect();
    let more = removed.len().saturating_sub(LISTED_PATHS);
    Some(format!(
        "{} path{} it names {} since removed, such as {}{}",
        removed.len(),
        if removed.len() == 1 { "" } else { "s" },
        if removed.len() == 1 { "was" } else { "were" },
        shown.join(", "),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    ))
}

/// A document's title: its frontmatter `title`, as MDX pages set it, else
/// its first heading.
fn title(source: &str, parsed: &markdown::Markdown) -> String {
    parsed
        .frontmatter
        .get("title")
        .filter(|t| !t.is_empty())
        .cloned()
        .or_else(|| {
            markdown::headings(source)
                .into_iter()
                .next()
                .map(|h| h.text)
        })
        .unwrap_or_default()
}

/// Version numbers such as `1.2.3` or `v0.20.4` in `text`.
fn versions(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for word in text.split(|c: char| !(c.is_ascii_digit() || c == '.')) {
        let parts: Vec<&str> = word.trim_matches('.').split('.').collect();
        if parts.len() == 3 && parts.iter().all(|p| !p.is_empty()) {
            found.push(parts.join("."));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::versions;

    #[test]
    fn versions_are_three_numbers() {
        assert_eq!(
            versions("docs/plans/2026-04-10-v0.20.4-bugfixes.md v1.2"),
            ["0.20.4"]
        );
        assert_eq!(versions("Release 1.7.1."), ["1.7.1"]);
    }
}
