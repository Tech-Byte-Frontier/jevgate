//! Security: packed function sources with presence Nouls per enabled rule,
//! then one trace follow-up per unit whose presence is not clear (which
//! statement, what kind, where its values come from, whether they are
//! handled), and for injection a recheck with up to three callers when the
//! origin stays unclear. A file's top-level setup statements are one more
//! unit for unsafe settings.
use super::{
    Asked, Block, Detail, FileContext, FilePlan, PACK_ITEMS, Planned, Presence, Questions, Settle,
    UnitPlan, compact, identity, pack, questions, unique_ids,
};
use crate::{
    analysis::{errors::CreatedError, sites::Site, units::Unit},
    catalog::{INJECTION, SENSITIVE_DATA, UNSAFE_SETTINGS},
    schema::Pass,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
    /// Errors it creates, with their message arguments.
    pub errors: &'a [CreatedError],
    pub lines: (usize, usize),
    /// Functions that call it, as (name, source), for the injection recheck.
    pub callers: Vec<(String, String)>,
    /// Enums its sites name, such as `ConfigKey` in `'${ConfigKey.aiTag}'`,
    /// defined in this or another selected file: fixed choices, not
    /// parameters, which the trace otherwise could not tell apart.
    pub enums: Vec<String>,
    /// C# constants it names, as `Class.Field = value`: a key written in
    /// the code or a value read from configuration.
    pub constants: Vec<String>,
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
    enums: &BTreeMap<String, String>,
    constants: &BTreeMap<String, Vec<String>>,
) -> Subject<'a> {
    let source = unit.source(file.source).to_string();
    Subject {
        name: unit.name.clone(),
        kind: "function",
        constants: named_constants(file, &source, constants),
        source,
        sites: &unit.sites,
        errors: &unit.errors,
        lines: (unit.line, unit.end_line),
        callers,
        enums: named_enums(&unit.sites, enums),
    }
}

/// Constant declarations shown with one subject, at most.
const CONSTANTS: usize = 4;

/// The declarations of the C# constants a C# subject names.
fn named_constants(
    file: &FileContext<'_>,
    source: &str,
    constants: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    if file.language != questions::CSHARP {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (field, declarations) in constants {
        let named = source.match_indices(field.as_str()).any(|(at, _)| {
            let before = source[..at].chars().next_back();
            let after = source[at + field.len()..].chars().next();
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            !word(before) && !word(after)
        });
        if named {
            found.extend(declarations.iter().cloned());
        }
    }
    found.truncate(CONSTANTS);
    found
}

/// Enum definitions shown with one subject, at most.
const ENUMS: usize = 3;

/// The definitions of enums named as `Name.member` or `Name::member` in the sites.
fn named_enums(sites: &[Site], enums: &BTreeMap<String, String>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for site in sites {
        for (name, definition) in enums {
            let named = site.text.match_indices(name.as_str()).any(|(at, _)| {
                let before = site.text[..at].chars().next_back();
                let after = &site.text[at + name.len()..];
                before.is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
                    && (after.starts_with('.') || after.starts_with("::"))
            });
            if named && found.len() < ENUMS && !found.contains(definition) {
                found.push(definition.clone());
            }
        }
    }
    found
}

pub(super) fn setup_subject<'a>(
    file: &FileContext<'_>,
    setup: &'a crate::analysis::sites::Setup,
    constants: &BTreeMap<String, Vec<String>>,
) -> Option<Subject<'a>> {
    let first = setup.statements.first()?;
    let last = setup.statements.last()?;
    let source: Vec<&str> = setup
        .statements
        .iter()
        .map(|(range, ..)| &file.source[range.clone()])
        .collect();
    let source = source.join("\n");
    Some(Subject {
        name: MODULE_SETUP.into(),
        kind: "module",
        constants: named_constants(file, &source, constants),
        source,
        sites: &setup.sites,
        errors: &[],
        lines: (first.1, last.2),
        callers: Vec::new(),
        enums: Vec::new(),
    })
}

/// The name of the unit that holds a file's top-level setup statements.
pub(super) const MODULE_SETUP: &str = "module setup";

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
    let settles = settles(file, subject, rule, id);
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
            messages: if rule == SENSITIVE_DATA {
                subject.errors.iter().map(|e| e.message.clone()).collect()
            } else {
                Vec::new()
            },
            trace,
            settles,
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
                if let Detail::Security { trace, settles, .. } = &mut unit.detail {
                    *trace = None;
                    settles.clear();
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

/// The specific checks a rule's trace can ask; the one that finds a concern
/// names the finding's kind. Checks of one language are answered only in
/// its files.
pub(super) fn checks(rule: &str) -> Vec<&'static questions::Check> {
    asked_checks(rule, questions::CSHARP)
}

/// The checks a rule's trace asks about a file in `language`.
fn asked_checks(rule: &str, language: &str) -> Vec<&'static questions::Check> {
    let (general, csharp): (&'static [questions::Check], &'static [questions::Check]) = match rule {
        INJECTION => (&questions::UNHANDLED, &questions::CSHARP_UNHANDLED),
        SENSITIVE_DATA => (&questions::EXPOSURES, &[]),
        _ => (&questions::WEAK_SETTINGS, &questions::CSHARP_SETTINGS),
    };
    let csharp = if language == questions::CSHARP {
        csharp
    } else {
        &[]
    };
    general.iter().chain(csharp).collect()
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
    // With the errors it creates listed, each message is asked about; a
    // function that creates none is asked about its messages as a whole.
    let messages: Vec<Value> = subject
        .errors
        .iter()
        .enumerate()
        .map(|(i, e)| json!({"id": format!("m{i}"), "error": e.error, "message": e.message}))
        .collect();
    if rule == SENSITIVE_DATA && messages.is_empty() {
        ask("own_messages", questions::security_own_messages(&code));
    } else if rule == SENSITIVE_DATA {
        let ids: Vec<String> = (0..messages.len()).map(|i| format!("m{i}")).collect();
        ask("messages", questions::security_message_origin(&ids));
    }
    for check in asked_checks(rule, file.language) {
        ask(check.id, check.body(&code));
    }
    let mut state = json!({
        "file": file.file_state(),
        subject.kind: {"name": subject.name, "source": subject.source},
        "sites": subject.sites.iter().map(|s| json!({"id": s.id, "source": s.text})).collect::<Vec<_>>(),
    });

    if rule == SENSITIVE_DATA && !messages.is_empty() {
        state["messages"] = json!(messages);
    }
    if rule == INJECTION && !subject.enums.is_empty() {
        state["enums_named_in_sites"] = json!(subject.enums);
    }
    if rule == UNSAFE_SETTINGS && !subject.constants.is_empty() {
        state["constants_named"] = json!(subject.constants);
    }

    file.request("trace", state, questions)
}

/// The origin question and the injection checks again, with the functions
/// that call it: callers show both where values come from and whether the
/// paths or text they pass are the program's own.
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
    for check in asked_checks(INJECTION, file.language) {
        questions.ask(
            check.id.into(),
            check.with_callers(&code),
            id,
            INJECTION,
            check.id,
            Pass::Recheck,
        );
    }
    let mut state = json!({
        "file": file.file_state(),
        subject.kind: {"name": subject.name, "source": subject.source},
        "callers": subject.callers.iter().map(|(name, source)| json!({"name": name, "source": source})).collect::<Vec<_>>(),
    });
    if !subject.enums.is_empty() {
        state["enums_named_in_sites"] = json!(subject.enums);
    }
    let (request, asked) = file.request("recheck", state, questions);
    file.budget.fits(&request).then_some((request, asked))
}

/// A Choice asked when one of a rule's checks stays undecided after the
/// trace and recheck; it can only clear the checks it settles, so it is
/// asked apart from them.
pub(in crate::units) struct SettleKind {
    pub rule: &'static str,
    /// The question id of the Choice.
    pub question: &'static str,
    /// The checks that call for it while undecided, and that it settles.
    pub checks: &'static [&'static str],
    /// The options whose combined probability at the threshold clears them.
    pub clears: &'static [&'static str],
    /// Whether the functions that call the subject are sent with it.
    callers: bool,
    pub when: SettleWhen,
}

/// Which units a settle Choice is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::units) enum SettleWhen {
    /// An uncertain unit whose checks stay undecided.
    Undecided,
    /// Also a consider or note that rests on its undecided checks: a note
    /// that a client component's fetch "places a parameter into a URL it
    /// requests" only puzzled readers.
    UndecidedOrFinding,
}

/// Every settle Choice. Where a URL comes from settles the URL check: on
/// clients of a fixed or configured service it split on a variable path or
/// query; so does code that runs only in the user's browser. Where a redirect leads, how markup is rendered and which origins
/// may send credentials settle theirs, which split on client components that
/// navigate to fixed paths or render values as attributes, and on route
/// handlers that answer preflights for any origin without credentials. What
/// its logs write settles a logged object, which split on errors caught from
/// a payment or database call. Where a function's text goes settles error
/// details (see `exposure_signal`), also under a finding that claims the text
/// likely reaches a client.
pub(in crate::units) const SETTLES: [SettleKind; 7] = [
    SettleKind {
        rule: INJECTION,
        question: "url_parts",
        checks: &["url"],
        clears: &questions::OWN_PARTS,
        callers: true,
        when: SettleWhen::Undecided,
    },
    SettleKind {
        rule: INJECTION,
        question: "runs_in",
        checks: &["url"],
        clears: &[questions::BROWSER],
        callers: false,
        when: SettleWhen::UndecidedOrFinding,
    },
    SettleKind {
        rule: INJECTION,
        question: "redirect_target",
        checks: &["redirect"],
        clears: &questions::OWN_TARGETS,
        callers: true,
        when: SettleWhen::Undecided,
    },
    SettleKind {
        rule: INJECTION,
        question: "markup_output",
        checks: &["markup"],
        clears: &questions::INERT_MARKUP,
        callers: false,
        when: SettleWhen::Undecided,
    },
    SettleKind {
        rule: SENSITIVE_DATA,
        question: "destination",
        checks: &["error_details", "exception_to_client"],
        clears: &questions::AWAY_FROM_CLIENTS,
        callers: false,
        when: SettleWhen::UndecidedOrFinding,
    },
    SettleKind {
        rule: SENSITIVE_DATA,
        question: "logged",
        checks: &["logs_object_secret"],
        clears: &questions::PLAIN_LOGS,
        callers: false,
        when: SettleWhen::Undecided,
    },
    SettleKind {
        rule: UNSAFE_SETTINGS,
        question: "cors_origins",
        checks: &["cors"],
        clears: &questions::SAFE_ORIGINS,
        callers: false,
        when: SettleWhen::Undecided,
    },
];

/// The settle follow-ups of one unit, one per Choice of its rule, each sent
/// only when its checks stay undecided.
fn settles(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    rule: &'static str,
    id: &str,
) -> Vec<Settle> {
    SETTLES
        .iter()
        .filter(|kind| kind.rule == rule)
        .filter_map(|kind| {
            let request = settle(file, subject, kind, id);
            file.budget.fits(&request.0).then_some(Settle {
                question: kind.question,
                request,
            })
        })
        .collect()
}

fn settle(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    kind: &SettleKind,
    id: &str,
) -> (Value, Asked) {
    let code = subject.code();
    let callers = kind.callers && !subject.callers.is_empty();
    let body = match kind.question {
        "url_parts" => questions::security_url_parts(&code, callers),
        "runs_in" => questions::security_runs_in(&code),
        "redirect_target" => questions::security_redirect_target(&code, callers),
        "markup_output" => questions::security_markup_output(&code),
        "destination" => questions::security_destination(&code),
        "logged" => questions::security_logged(&code),
        _ => questions::security_cors_origins(&code),
    };
    let mut questions = Questions::default();
    questions.ask(
        kind.question.into(),
        body,
        id,
        kind.rule,
        kind.question,
        Pass::Settle,
    );
    let mut state = json!({
        "file": file.file_state(),
        subject.kind: {"name": subject.name, "source": subject.source},
    });
    if callers {
        state["callers"] = json!(
            subject
                .callers
                .iter()
                .map(|(name, source)| json!({"name": name, "source": source}))
                .collect::<Vec<_>>()
        );
    }
    file.request("settle", state, questions)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_naming_an_enum_carry_its_definition() {
        let site = |text: &str| Site {
            id: "S1".into(),
            text: text.into(),
            line: 1,
            end_line: 1,
        };
        let enums: BTreeMap<String, String> = [
            (
                "ConfigKey".to_string(),
                "export enum ConfigKey {\n  aiTag = 'ai_tag',\n}".to_string(),
            ),
            ("Key".to_string(), "enum Key { A }".to_string()),
        ]
        .into();
        let sites = [site(
            "`INSERT INTO stores (key, value) VALUES ('${ConfigKey.aiTag}', ?)`",
        )];
        assert_eq!(named_enums(&sites, &enums), [enums["ConfigKey"].clone()]);
        assert!(named_enums(&[site("`SELECT ${MyConfigKey.x} ${key}`")], &enums).is_empty());
    }
}
