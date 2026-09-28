//! Security: packed function sources with presence Nouls per enabled rule,
//! then one trace follow-up per unit whose presence is not clear (which
//! statement, what kind, where its values come from, whether they are
//! handled), and for injection a recheck with up to three callers when the
//! origin stays unclear. A file's top-level setup statements are one more
//! unit for unsafe settings; a PHP file's top-level statements are a page
//! script, judged like a function by every rule. `subject` builds what each
//! unit is judged on and `settle` holds the Choices that settle an undecided
//! check.
use super::{
    Asked, Block, Confirms, Detail, FileContext, FilePlan, Planned, Presence, Questions, Settle,
    UnitPlan, compact, identity, pack_runs, questions, unique_ids,
};
use crate::{
    analysis::{errors::CreatedError, sites::Site, units::Unit},
    catalog::{INJECTION, SENSITIVE_DATA, UNSAFE_SETTINGS},
    schema::Pass,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

mod settle;
mod subject;
pub(in crate::units) use settle::*;
pub(in crate::units) use subject::*;

/// The presence questions of each rule, in the order they are asked.
pub(super) const PRESENCE: [(&str, &[&str]); 3] = [
    (INJECTION, &["interpreted", "resource"]),
    (SENSITIVE_DATA, &["logs_secret", "error_details"]),
    (UNSAFE_SETTINGS, &["weakened"]),
];

/// Callers shown when the origin of an injection's values stays unclear.
pub(super) const CALLERS: usize = 3;

/// Plan every enabled rule's units for these subjects. Functions and a PHP
/// page script are packed; the module setup, when present, is judged for
/// unsafe settings only. `django` marks Django code, whose presence
/// questions name its calls.
pub(super) fn plan(
    file: &FileContext<'_>,
    functions: &[Subject<'_>],
    setup: Option<Subject<'_>>,
    rules: &[&'static str],
    django: bool,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (script, setup) = match setup {
        Some(subject) if subject.name == SCRIPT => (Some(subject), None),
        other => (None, other),
    };
    let judged: Vec<&Subject<'_>> = functions
        .iter()
        .chain(&script)
        .filter(|s| !s.sites.is_empty())
        .collect();
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
        items.push((units, subject.state()));
    }
    // Runs end after subjects' names, so a function added, removed or
    // resized re-asks only its own run.
    for group in pack_runs(
        items,
        |(_, state)| state["name"].as_str().unwrap_or_default(),
        |(_, state)| state,
    ) {
        send(file, group, "functions", django, out, requests);
    }
    if let Some(setup) = setup.filter(|s| !s.sites.is_empty())
        && rules.contains(&UNSAFE_SETTINGS)
    {
        let id = format!("{}:module", prefix(UNSAFE_SETTINGS));
        let unit = push_unit(file, out, &setup, UNSAFE_SETTINGS, &id);
        let mut state = setup.state();
        if let Some(object) = state.as_object_mut() {
            object.remove("name");
        }
        send(
            file,
            vec![(vec![unit], state)],
            "module",
            django,
            out,
            requests,
        );
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
    let confirm = (rule == INJECTION)
        .then(|| confirm(file, subject, id))
        .flatten();
    let checked = (rule == INJECTION)
        .then(|| confirm_checks(file, subject, id))
        .flatten();
    let queried = (rule == INJECTION)
        .then(|| confirm_query(file, subject, id))
        .flatten();
    let rendered = (rule == UNSAFE_SETTINGS)
        .then(|| confirm_html(file, subject, id))
        .flatten();
    let readers = (rule == SENSITIVE_DATA)
        .then(|| confirm_readers(file, subject, id))
        .flatten();
    let logging = (rule == SENSITIVE_DATA)
        .then(|| confirm_logging(file, subject, id))
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
            trace: trace.map(Into::into),
            settles,
            confirms: Box::new(Confirms {
                values: confirm.map(Into::into),
                checked: checked.map(Into::into),
                queried: queried.map(Into::into),
                rendered: rendered.map(Into::into),
                readers: readers.map(Into::into),
                logging: logging.map(Into::into),
            }),
            django: subject.django,
            test_path: subject.test_path,
        },
        recheck: recheck.map(Into::into),
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
    django: bool,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let (request, asked) = presence_request(file, &group, key, django);
    if file.budget.fits(&request) {
        requests.push(Planned {
            owner: file.owner,
            request,
            asked,
        });
        return;
    }
    for item in group {
        let (request, asked) = presence_request(file, std::slice::from_ref(&item), key, django);
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
                if let Detail::Security {
                    trace,
                    settles,
                    confirms,
                    ..
                } = &mut unit.detail
                {
                    *trace = None;
                    settles.clear();
                    **confirms = Confirms::default();
                }
            }
        }
    }
}

fn presence_request(
    file: &FileContext<'_>,
    items: &[Item],
    key: &str,
    django: bool,
) -> (Value, Asked) {
    let mut questions = Questions::default();
    for (index, (units, _)) in items.iter().enumerate() {
        let code = if key == "module" {
            "module.source".to_string()
        } else {
            format!("functions[{index}].source")
        };
        let source = items[index].1["source"].as_str().unwrap_or_default();
        let deserializers = questions::deserializers_named(file.language, source);
        let xml = questions::parses_xml(file.source, source);
        let rendered = !django && items[index].1.get(RENDERED).is_some();
        for (rule, _, id) in units {
            for question in presence_questions(rule) {
                questions.ask(
                    format!("{}{index}_{question}", &key[..1]),
                    presence_body(question, &code, (django, rendered), (deserializers, xml)),
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

/// `named` holds the deserializers and whether an XML parser that can
/// resolve entities appear in the source.
fn presence_body(
    question: &str,
    code: &str,
    (django, rendered): (bool, bool),
    (deserializers, xml): (Option<&str>, bool),
) -> Value {
    match question {
        "interpreted" => {
            questions::security_interpreted(code, django, rendered, deserializers, xml)
        }
        "resource" => questions::security_resource(code, django),
        "logs_secret" => questions::security_logs_secret(code),
        "error_details" => questions::security_error_details(code, django),
        _ => questions::security_weakened(code, django),
    }
}

/// The specific checks a rule's trace can ask; the one that finds a concern
/// names the finding's kind. Checks of one language or framework are
/// answered only in its files.
pub(super) fn checks(rule: &str) -> Vec<&'static questions::Check> {
    let (.., php) = rule_checks(rule);
    let mut all = asked_checks(rule, questions::CSHARP, false, "", false);
    // Django and PHP each ask a `deserialize` check of their own: one kind.
    // The XML check is asked only of source that names an XML parser.
    let xml = (rule == INJECTION).then_some(&questions::XXE);
    for check in asked_checks(rule, "", true, "", false)
        .into_iter()
        .chain(php)
        .chain(xml)
    {
        if !all.iter().any(|c| c.id == check.id) {
            all.push(check);
        }
    }
    all
}

/// A rule's general checks, and those of C# files, Django code and PHP files.
fn rule_checks(
    rule: &str,
) -> (
    &'static [questions::Check],
    &'static [questions::Check],
    &'static [questions::Check],
    &'static [questions::Check],
) {
    match rule {
        INJECTION => (
            &questions::UNHANDLED,
            &questions::CSHARP_UNHANDLED,
            &questions::DJANGO_UNHANDLED,
            &questions::PHP_UNHANDLED,
        ),
        SENSITIVE_DATA => (
            &questions::EXPOSURES,
            &[],
            &questions::DJANGO_EXPOSURES,
            &[],
        ),
        _ => (
            &questions::WEAK_SETTINGS,
            &questions::CSHARP_SETTINGS,
            &questions::DJANGO_SETTINGS,
            &[],
        ),
    }
}

/// The checks a rule's trace asks about `source` in a file in `language`;
/// Django code (`django`) is asked the Django variant of a check where one
/// exists, and the Django checks besides; PHP files are asked PHP's own
/// checks only of source that names what they ask about, and other code
/// the deserialize check of its language when its source names one of the
/// language's deserializers. Code that parses XML with a parser able to
/// resolve external entities (`xml`) is asked the XML check. The token and
/// key checks are asked of code outside C# and Django, which ask their own.
fn asked_checks(
    rule: &str,
    language: &str,
    django: bool,
    source: &str,
    xml: bool,
) -> Vec<&'static questions::Check> {
    let (general, csharp, framework, php) = rule_checks(rule);
    let csharp = if language == questions::CSHARP {
        csharp
    } else {
        &[]
    };
    let framework = if django { framework } else { &[] };
    let php = php
        .iter()
        .filter(|check| language == questions::PHP && questions::php_mentions(check.id, source));
    general
        .iter()
        .map(|check| {
            questions::DJANGO_VARIANTS
                .iter()
                .find(|variant| django && variant.id == check.id)
                .unwrap_or(check)
        })
        .chain(csharp)
        .chain(framework)
        .chain(php)
        .chain(
            (rule == INJECTION && !django)
                .then(|| questions::deserializer_check(language, source))
                .flatten(),
        )
        .chain((rule == INJECTION && xml).then_some(&questions::XXE))
        .chain(
            (rule == UNSAFE_SETTINGS && language != questions::CSHARP && !django)
                .then_some(&questions::TOKEN_AND_KEY)
                .into_iter()
                .flatten(),
        )
        .collect()
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
    // Other code keeps the key the site question has always named.
    let listed = if subject.django {
        code.as_str()
    } else {
        "function.source"
    };
    ask("site", questions::security_site(what, &ids, listed));
    if rule == INJECTION {
        ask(
            "origin",
            questions::security_origin(&code, false, subject.django),
        );
    } else {
        ask(
            "dev_only",
            questions::security_dev_only(&code, subject.django),
        );
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
    }
    // With the errors its callees create in view, the exception check asks
    // whose text a response carries, not who raised it.
    let from_callees =
        rule == SENSITIVE_DATA && !subject.django && !subject.callee_errors.is_empty();
    if rule == SENSITIVE_DATA && !messages.is_empty() {
        let ids: Vec<String> = (0..messages.len()).map(|i| format!("m{i}")).collect();
        ask(
            "messages",
            questions::security_message_origin(&ids, from_callees),
        );
    }
    for check in trace_checks(file, subject, rule, from_callees) {
        ask(check.id, check.body(&code));
    }
    file.request(
        "trace",
        trace_state(file, subject, rule, messages),
        questions,
    )
}

/// The checks a unit's trace and recheck ask, in the variants its evidence
/// calls for: the exception check of text its callees create, and the
/// markup check of a function that renders unescaped templates.
fn trace_checks(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    rule: &'static str,
    from_callees: bool,
) -> Vec<&'static questions::Check> {
    let xml = questions::parses_xml(file.source, &subject.source);
    asked_checks(rule, file.language, subject.django, &subject.source, xml)
        .into_iter()
        // A template's code writes its values unescaped by construction,
        // which injection judges; it turns no escaping setting off. Asked
        // anyway, a JSP page's `<%= … %>` read as one.
        .filter(|check| !(subject.name == TEMPLATE_CODE && check.id == "escape"))
        .map(|check| {
            if from_callees && check.id == "exception_to_client" {
                &questions::EXCEPTION_TO_CLIENT_FROM_CALLEES
            } else if subject.renders() && check.id == "markup" {
                &questions::VIEW_MARKUP
            } else {
                check
            }
        })
        .collect()
}

/// A trace's state: the unit's source with its sites, and the evidence its
/// rule's checks read.
fn trace_state(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    rule: &str,
    messages: Vec<Value>,
) -> Value {
    let mut state = json!({
        "file": file.file_state(),
        subject.kind: subject.state(),
        "sites": subject.sites.iter().map(|s| json!({"id": s.id, "source": s.text})).collect::<Vec<_>>(),
    });
    if rule == SENSITIVE_DATA && !messages.is_empty() {
        state["messages"] = json!(messages);
    }
    if rule == SENSITIVE_DATA && !subject.callee_errors.is_empty() {
        state["errors_created_by_functions_it_calls"] = json!(subject.callee_errors);
    }
    if rule == INJECTION && !subject.enums.is_empty() {
        state["enums_named_in_sites"] = json!(subject.enums);
    }
    if rule == UNSAFE_SETTINGS && !subject.constants.is_empty() {
        state["constants_named"] = json!(subject.constants);
    }
    state
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
        questions::security_origin(&code, true, subject.django),
        id,
        INJECTION,
        "origin",
        Pass::Recheck,
    );
    for check in trace_checks(file, subject, INJECTION, false) {
        questions.ask(
            check.id.into(),
            check.with_callers(&code),
            id,
            INJECTION,
            check.id,
            Pass::Recheck,
        );
    }
    let (request, asked) = file.request("recheck", with_callers(file, subject), questions);
    file.budget.fits(&request).then_some((request, asked))
}

/// The unit's code with the functions that call it and the enums its sites
/// name: the evidence of its recheck and of what its values can hold.
fn with_callers(file: &FileContext<'_>, subject: &Subject<'_>) -> Value {
    let mut state = json!({
        "file": file.file_state(),
        subject.kind: subject.state(),
    });
    if !subject.callers.is_empty() {
        state["callers"] = json!(
            subject
                .callers
                .iter()
                .map(|(name, source)| json!({"name": name, "source": source}))
                .collect::<Vec<_>>()
        );
    }
    if !subject.enums.is_empty() {
        state["enums_named_in_sites"] = json!(subject.enums);
    }
    state
}

/// What the values an injection consider rests on can hold, asked only
/// when its origin was the function's parameters: the function, and the
/// functions that call it.
fn confirm(file: &FileContext<'_>, subject: &Subject<'_>, id: &str) -> Option<(Value, Asked)> {
    let code = subject.code();
    let mut questions = Questions::default();
    questions.ask(
        "values".into(),
        questions::injection_values(&code, !subject.callers.is_empty()),
        id,
        INJECTION,
        "values",
        Pass::Locate,
    );
    let (request, asked) = file.request("locate", with_callers(file, subject), questions);
    file.budget.fits(&request).then_some((request, asked))
}

/// What the values of a path, markup or redirect finding can hold or where
/// they lead, asked only after a finding whose one concern is one of those;
/// only the question of its kind is read. The function, the functions that
/// call it and the project's types its parameters name.
fn confirm_checks(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    id: &str,
) -> Option<(Value, Asked)> {
    let code = subject.code();
    let callers = !subject.callers.is_empty();
    let mut questions = Questions::default();
    for (question, body) in [
        (
            "paths",
            questions::injection_paths(&code, callers, !subject.types.is_empty()),
        ),
        ("markup_values", questions::markup_values(&code, callers)),
        ("redirect_reach", questions::redirect_reach(&code, callers)),
    ] {
        questions.ask(question.into(), body, id, INJECTION, question, Pass::Locate);
    }
    let mut state = with_callers(file, subject);
    if !subject.types.is_empty() {
        state["types_named_in_parameters"] = json!(subject.types);
    }
    let (request, asked) = file.request("locate", state, questions);
    file.budget.fits(&request).then_some((request, asked))
}

/// What the values of an SQL, command or code finding hold where they enter
/// it, asked only after a finding whose one concern is one of those: the
/// function, the functions that call it and the project's types its
/// parameters name.
fn confirm_query(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    id: &str,
) -> Option<(Value, Asked)> {
    let code = subject.code();
    let types = !subject.types.is_empty();
    let mut questions = Questions::default();
    questions.ask(
        "query_values".into(),
        questions::query_values(&code, !subject.callers.is_empty(), types),
        id,
        INJECTION,
        "query_values",
        Pass::Locate,
    );
    let mut state = with_callers(file, subject);
    if types {
        state["types_named_in_parameters"] = json!(subject.types);
    }
    let (request, asked) = file.request("locate", state, questions);
    file.budget.fits(&request).then_some((request, asked))
}

/// What the HTML a weak-settings finding writes without escaping holds,
/// asked only after a finding its escaping check raised: the function alone.
fn confirm_html(file: &FileContext<'_>, subject: &Subject<'_>, id: &str) -> Option<(Value, Asked)> {
    let body = questions::raw_html(&subject.code());
    confirm_alone(file, subject, id, (UNSAFE_SETTINGS, "raw_html", body), None)
}

/// Who reads the error text of an error-detail finding, asked only after
/// such a finding: the function and the opening of the project's README.
fn confirm_readers(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    id: &str,
) -> Option<(Value, Asked)> {
    let body = questions::error_readers(&subject.code(), file.project.is_some());
    let opening = file
        .project
        .map(|opening| ("project", json!({"readme_opening": opening})));
    confirm_alone(
        file,
        subject,
        id,
        (SENSITIVE_DATA, "error_readers", body),
        opening,
    )
}

/// When the log line of a logging finding runs, asked only after such a
/// finding: the function alone.
fn confirm_logging(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    id: &str,
) -> Option<(Value, Asked)> {
    let body = questions::logged_when(&subject.code());
    confirm_alone(
        file,
        subject,
        id,
        (SENSITIVE_DATA, "logged_when", body),
        None,
    )
}

/// One Choice of `rule` asked after a finding, with the function as its
/// evidence and `extra` state beside it.
fn confirm_alone(
    file: &FileContext<'_>,
    subject: &Subject<'_>,
    id: &str,
    (rule, question, body): (&'static str, &'static str, Value),
    extra: Option<(&str, Value)>,
) -> Option<(Value, Asked)> {
    let mut questions = Questions::default();
    questions.ask(question.into(), body, id, rule, question, Pass::Locate);
    let mut state = json!({
        "file": file.file_state(),
        subject.kind: subject.state(),
    });
    if let Some((key, value)) = extra {
        state[key] = value;
    }
    let (request, asked) = file.request("locate", state, questions);
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
