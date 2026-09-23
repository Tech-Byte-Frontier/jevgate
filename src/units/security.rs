//! Security: packed function sources with presence Nouls per enabled rule,
//! then one trace follow-up per unit whose presence is not clear (which
//! statement, what kind, where its values come from, whether they are
//! handled), and for injection a recheck with up to three callers when the
//! origin stays unclear. A file's top-level setup statements are one more
//! unit for unsafe settings.
use super::{
    Asked, Block, Detail, FileContext, FilePlan, PACK_ITEMS, Planned, Presence, Questions,
    UnitPlan, compact, identity, pack, questions, unique_ids,
};
use crate::{
    analysis::{sites::Site, units::Unit},
    catalog::{INJECTION, SENSITIVE_DATA, UNSAFE_SETTINGS},
    schema::Pass,
};
use serde_json::{Value, json};

/// The presence questions of each rule, in the order they are asked.
pub(super) const PRESENCE: [(&str, &[&str]); 3] = [
    (INJECTION, &["interpreted", "resource"]),
    (SENSITIVE_DATA, &["logs_secret", "error_details"]),
    (UNSAFE_SETTINGS, &["weakened"]),
];

/// Callers shown when the origin of an injection's values stays unclear.
pub(super) const CALLERS: usize = 3;

/// A function or setup statements that the security rules judge.
pub(super) struct Subject<'a> {
    pub name: String,
    /// `function` or `module`: the state key and the source path.
    pub kind: &'static str,
    pub source: String,
    pub sites: &'a [Site],
    pub lines: (usize, usize),
    /// Functions that call it, as (name, source), for the injection recheck.
    pub callers: Vec<(String, String)>,
}

impl Subject<'_> {
    fn code(&self) -> String {
        format!("{}.source", self.kind)
    }
}

pub(super) fn function_subject<'a>(
    file: &FileContext<'_>,
    unit: &'a Unit,
    callers: Vec<(String, String)>,
) -> Subject<'a> {
    Subject {
        name: unit.name.clone(),
        kind: "function",
        source: unit.source(file.source).to_string(),
        sites: &unit.sites,
        lines: (unit.line, unit.end_line),
        callers,
    }
}

pub(super) fn setup_subject<'a>(
    file: &FileContext<'_>,
    setup: &'a crate::analysis::sites::Setup,
) -> Option<Subject<'a>> {
    let first = setup.statements.first()?;
    let last = setup.statements.last()?;
    let source: Vec<&str> = setup
        .statements
        .iter()
        .map(|(range, ..)| &file.source[range.clone()])
        .collect();
    Some(Subject {
        name: "module setup".into(),
        kind: "module",
        source: source.join("\n"),
        sites: &setup.sites,
        lines: (first.1, last.2),
        callers: Vec::new(),
    })
}

/// Plan every enabled rule's units for these subjects. Functions are packed;
/// the module setup, when present, is judged for unsafe settings only.
pub(super) fn plan(
    file: &FileContext<'_>,
    functions: &[Subject<'_>],
    setup: Option<Subject<'_>>,
    rules: &[&'static str],
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let judged: Vec<&Subject<'_>> = functions.iter().filter(|s| !s.sites.is_empty()).collect();
    let ids: Vec<Vec<String>> = rules
        .iter()
        .map(|rule| unique_ids(prefix(rule), judged.iter().map(|s| s.name.as_str())))
        .collect();
    let mut items = Vec::new();
    for (index, subject) in judged.iter().enumerate() {
        let units = rules
            .iter()
            .zip(&ids)
            .map(|(rule, ids)| push_unit(file, out, subject, rule, &ids[index]))
            .collect();
        items.push((
            units,
            json!({"name": subject.name, "source": subject.source}),
        ));
    }
    for group in pack(items, PACK_ITEMS, |(_, state)| state) {
        send(file, group, "functions", out, requests);
    }
    if let Some(setup) = setup.filter(|s| !s.sites.is_empty())
        && rules.contains(&UNSAFE_SETTINGS)
    {
        let id = format!("{}:module", prefix(UNSAFE_SETTINGS));
        let unit = push_unit(file, out, &setup, UNSAFE_SETTINGS, &id);
        let state = json!({"source": setup.source});
        send(file, vec![(vec![unit], state)], "module", out, requests);
    }
}

fn prefix(rule: &str) -> &'static str {
    match rule {
        INJECTION => "injection",
        SENSITIVE_DATA => "data",
        _ => "settings",
    }
}

/// One rule's unit for a subject, with its trace and (for injection) recheck
/// follow-ups; returns (rule, unit index, id).
fn push_unit(
    file: &FileContext<'_>,
    out: &mut FilePlan,
    subject: &Subject<'_>,
    rule: &'static str,
    id: &str,
) -> (&'static str, usize, String) {
    let trace =
        Some(trace(file, subject, rule, id)).filter(|(request, _)| file.budget.fits(request));
    let recheck = (rule == INJECTION)
        .then(|| recheck(file, subject, id))
        .flatten();
    out.units.push(UnitPlan {
        rule,
        id: id.to_string(),
        name: subject.name.clone(),
        presence: Presence::Judged,
        locations: vec![file.location(subject.lines.0, subject.lines.1, Some(&subject.name))],
        quote: None,
        lines: subject.lines.1 + 1 - subject.lines.0,
        identity: identity(&[&subject.name, &compact(&subject.source)]),
        detail: Detail::Security {
            sites: sites(file, subject),
            trace,
        },
        recheck,
    });
    (rule, out.units.len() - 1, id.to_string())
}

type Item = (Vec<(&'static str, usize, String)>, Value);

/// A pack that is too large is sent one subject at a time; a subject that
/// still does not fit needs context.
fn send(
    file: &FileContext<'_>,
    group: Vec<Item>,
    key: &str,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (request, asked) = presence_request(file, &group, key);
    if file.budget.fits(&request) {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
        return;
    }
    for item in group {
        let (request, asked) = presence_request(file, std::slice::from_ref(&item), key);
        if file.budget.fits(&request) {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        } else {
            for (_, index, _) in &item.0 {
                let unit = &mut out.units[*index];
                unit.presence = Presence::NeedsContext;
                unit.recheck = None;
                if let Detail::Security { trace, .. } = &mut unit.detail {
                    *trace = None;
                }
            }
        }
    }
}

fn presence_request(file: &FileContext<'_>, items: &[Item], key: &str) -> (Value, Asked) {
    let mut questions = Questions::default();
    for (index, (units, _)) in items.iter().enumerate() {
        let code = if key == "module" {
            "module.source".to_string()
        } else {
            format!("functions[{index}].source")
        };
        for (rule, _, id) in units {
            for question in presence_questions(rule) {
                questions.ask(
                    format!("{}{index}_{question}", &key[..1]),
                    presence_body(question, &code),
                    id,
                    rule,
                    question,
                    Pass::First,
                );
            }
        }
    }
    let state = if key == "module" {
        json!({"file": file.file_state(), "module": items[0].1})
    } else {
        json!({
            "file": file.file_state(),
            "functions": items.iter().map(|(_, state)| state.clone()).collect::<Vec<_>>(),
        })
    };
    file.request("security", state, questions)
}

pub(super) fn presence_questions(rule: &str) -> &'static [&'static str] {
    PRESENCE
        .iter()
        .find(|(r, _)| *r == rule)
        .map_or(&[], |(_, questions)| questions)
}

fn presence_body(question: &str, code: &str) -> Value {
    match question {
        "interpreted" => questions::security_interpreted(code),
        "resource" => questions::security_resource(code),
        "logs_secret" => questions::security_logs_secret(code),
        "error_details" => questions::security_error_details(code),
        _ => questions::security_weakened(code),
    }
}

/// The specific checks a rule's trace asks; the one that finds a concern
/// names the finding's kind.
pub(super) fn checks(rule: &str) -> &'static [questions::Check] {
    match rule {
        INJECTION => &questions::UNHANDLED,
        SENSITIVE_DATA => &questions::EXPOSURES,
        _ => &questions::WEAK_SETTINGS,
    }
}

/// The trace follow-up of one unit: which site, the rule's specific checks,
/// and for injection where the values come from; for the other rules
/// whether the code runs only in development.
fn trace(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    rule: &'static str,
    id: &str,
) -> (Value, Asked) {
    let code = subject.code();
    let ids: Vec<String> = subject.sites.iter().map(|s| s.id.clone()).collect();
    let what = match rule {
        INJECTION => "places a variable into a query, command, code, markup, file path or URL",
        SENSITIVE_DATA => "logs a secret or personal value, or sends internal error details",
        _ => "turns off a security check or chooses a weak setting",
    };
    let mut questions = Questions::default();
    let mut ask = |question: &'static str, body: Value| {
        questions.ask(question.into(), body, id, rule, question, Pass::Trace);
    };
    ask("site", questions::security_site(what, &ids));
    if rule == INJECTION {
        ask("origin", questions::security_origin(&code, false));
    } else {
        ask("dev_only", questions::security_dev_only(&code));
    }
    for check in checks(rule) {
        ask(check.id, check.body(&code));
    }
    let state = json!({
        "file": file.file_state(),
        subject.kind: {"name": subject.name, "source": subject.source},
        "sites": subject.sites.iter().map(|s| json!({"id": s.id, "source": s.text})).collect::<Vec<_>>(),
    });
    file.request("trace", state, questions)
}

/// The origin question again with the functions that call it.
fn recheck(file: &FileContext<'_>, subject: &Subject<'_>, id: &str) -> Option<(Value, Asked)> {
    if subject.callers.is_empty() {
        return None;
    }
    let code = subject.code();
    let mut questions = Questions::default();
    questions.ask(
        "origin".into(),
        questions::security_origin(&code, true),
        id,
        INJECTION,
        "origin",
        Pass::Recheck,
    );
    let state = json!({
        "file": file.file_state(),
        subject.kind: {"name": subject.name, "source": subject.source},
        "callers": subject.callers.iter().map(|(name, source)| json!({"name": name, "source": source})).collect::<Vec<_>>(),
    });
    let (request, asked) = file.request("recheck", state, questions);
    file.budget.fits(&request).then_some((request, asked))
}

fn sites(file: &FileContext<'_>, subject: &Subject<'_>) -> Vec<Block> {
    subject
        .sites
        .iter()
        .map(|site| Block {
            id: site.id.clone(),
            location: file.location(site.line, site.end_line, Some(&subject.name)),
        })
        .collect()
}
