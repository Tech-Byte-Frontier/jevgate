//! Planning every rule's units and requests over the selected scope: parse the
//! files once, build the facts that span files, then plan each file.
/// A scope of files sharing clone, subject and caller evidence.
pub(super) struct Scope<'a> {
    pub(super) owners: Vec<usize>,
    pub(super) inputs: &'a [Input],
    pub(super) views: &'a BTreeMap<usize, View>,
    pub(super) units: BTreeMap<usize, FileUnits>,
    pub(super) context: Vec<(PathBuf, &'a str, FileUnits)>,
    /// Agent instruction files, judged by the documentation rules only.
    documents: Vec<usize>,
    /// SQL and workflow files, judged by the access-control and workflow rules.
    configuration: Vec<usize>,
    /// Files only custom `file` and `hunk` questions read: source files
    /// without a parser (`true`) and text files their `paths` name.
    texts: Vec<(usize, bool)>,
}

use super::{
    Detail, FileContext, FilePlan, Plan, Planned, UnitPlan, access, custom, documents, drift,
    handlers, instructions, workflows,
};

use crate::{
    analysis::units::{self as parsed, FileUnits, Unit},
    catalog,
    file_kind::View,
    inventory::Input,
    options::CheckArgs,
    token_budget::{Limits, TokenBudget},
};

use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
};

mod file;
mod laws;
mod left_out;
mod security_units;
mod settings_modules;
mod shared;
mod trace_evidence;

use file::{java, plan_file};
use security_units::plan_security;
use shared::Shared;

impl Scope<'_> {
    pub(super) fn test_lines(&self, owner: usize) -> Vec<Range<usize>> {
        self.views[&owner]
            .test_lines
            .iter()
            .map(|r| r.start_line..r.end_line + 1)
            .collect()
    }

    /// Callable units outside tests, in selected files and explicit context,
    /// with the path and source of their file.
    pub(super) fn scope_units(&self) -> impl Iterator<Item = (&Path, &str, &Unit)> {
        let selected = self.owners.iter().flat_map(move |owner| {
            let lines = self.test_lines(*owner);
            let input = &self.inputs[*owner];
            let (path, source) = (
                input.result.path.as_path(),
                input.source.as_deref().unwrap_or(""),
            );
            self.units[owner]
                .units
                .iter()
                .filter(move |u| u.callable() && !lines.iter().any(|l| u.overlaps(l)))
                .map(move |u| (path, source, u))
        });
        let context = self.context.iter().flat_map(|(path, source, units)| {
            units
                .units
                .iter()
                .filter(|u| u.callable())
                .map(move |u| (path.as_path(), *source, u))
        });
        selected.chain(context)
    }

    /// `scope_units` of the languages with analyzers of their own, which
    /// the cross-file evidence of those languages reads: units of the
    /// generic tier (`analysis::generic`) never change their requests.
    pub(super) fn specific_units(&self) -> impl Iterator<Item = (&Path, &str, &Unit)> {
        self.scope_units()
            .filter(|(path, _, _)| crate::analysis::generic::of(path).is_none())
    }
}

/// Plan every selected file's units; `root` is the repository, whose README
/// says whether its comments are written for learners, and whose Git history
/// gives custom questions their changed hunks.
pub fn plan(
    inputs: &[Input],
    views: &BTreeMap<usize, View>,
    args: &CheckArgs,
    budget: &TokenBudget,
    root: &std::path::Path,
) -> Plan {
    let unanswered = |request: &serde_json::Value| crate::requests::unanswered(root, args, request);
    let budget = Limits::new(budget, &unanswered);
    let mut result = Plan::default();
    let mut custom = custom::Planner::new(args, root);
    let scope = parsed_scope(inputs, views, &mut result.skipped, &custom);
    let mut shared = Shared::new(&scope, args);
    shared.teaching = crate::docs::teaching(root);
    shared.laravel = root.join("artisan").is_file();
    for &owner in &scope.owners {
        let requests = (&mut result.requests, &mut custom);
        let file = plan_file(&scope, &shared, owner, args, budget, requests);
        result.files.insert(owner, file);
    }
    if shared.enabled(catalog::SENSITIVE_DATA) {
        let evidence = handlers::Evidence {
            links: &shared.links,
            hashes: &shared.hashes,
        };
        handlers::plan(&scope, &evidence, args, budget, &mut result);
    }
    let drift = drift::Shared::new(
        inputs,
        &scope.documents,
        args.enabled(catalog::DOC_STALENESS),
        args.enabled(catalog::DOC_DUPLICATION),
    );
    let sql: Vec<(usize, &Input)> = scope
        .configuration
        .iter()
        .filter(|&&o| inputs[o].result.role != crate::inventory::WORKFLOW)
        .map(|&o| (o, &inputs[o]))
        .collect();
    access::plan(&sql, args, budget, &mut result.files, &mut result.requests);
    plan_workflows(&scope, args, budget, &mut result);
    skip_left_out(&scope, &mut result);
    for &owner in &scope.documents {
        let file = plan_document(
            (owner, &inputs[owner]),
            args,
            budget,
            &drift,
            (&mut result.requests, &mut custom),
        );
        result.files.insert(owner, file);
    }
    for &(owner, source) in &scope.texts {
        let input = &inputs[owner];
        let text = input.source.as_deref().unwrap_or("");
        let language = custom::language(&input.result.path);
        let context = FileContext::plain((owner, input), (language, text), args, budget);
        let mut file = FilePlan {
            path: input.result.path.clone(),
            ..Default::default()
        };
        custom.text(&context, source, &mut file, &mut result.requests);
        result.files.insert(owner, file);
    }
    keep_changed(inputs, &mut result);
    // Text addressed to a reviewer is asked about once the requests that
    // send it are known: with `--base`, those the change touched.
    for &owner in &scope.owners {
        file::plan_steering(&scope, owner, args, budget, &mut result);
    }
    for &owner in &scope.documents {
        file::plan_document_steering(&scope, owner, args, budget, &mut result);
    }
    for &(owner, _) in &scope.texts {
        file::plan_text_steering(&scope, owner, args, budget, &mut result);
    }
    result
}

/// With `--base` judging what a change touched, drop the units it did not
/// touch and the requests asking only about them, so they are neither paid
/// for nor reported. A unit stays when one of its locations lies on lines
/// the change touched in that location's file: a copy pair stays when
/// either copy changed, and a copy in a file the check did not select is
/// unchanged. Units of a file judged whole all stay. So does only the code
/// the parser could not read where the change touched it: a grammar gap in
/// a function the change left alone was named on every change to its file.
fn keep_changed(inputs: &[Input], plan: &mut Plan) {
    let changes: BTreeMap<&Path, Option<&crate::revision::FileChange>> = inputs
        .iter()
        .map(|input| (input.result.path.as_path(), input.changed.as_ref()))
        .collect();
    let touched = |location: &crate::schema::Location| match changes.get(location.path.as_path()) {
        Some(Some(change)) => change.lines.touch(location.start_line, location.end_line),
        Some(None) => true,
        None => false,
    };
    let mut kept = BTreeMap::<usize, std::collections::BTreeSet<String>>::new();
    for (&owner, file) in &mut plan.files {
        let Some(change) = &inputs[owner].changed else {
            continue;
        };
        file.units
            .retain(|unit| chosen_when_planned(unit) || unit.locations.iter().any(touched));
        file.left_out
            .retain(|code| change.lines.touch(code.start_line, code.end_line));
        kept.insert(owner, file.units.iter().map(|u| u.id.clone()).collect());
    }
    plan.requests.retain(|request| {
        kept.get(&request.owner).is_none_or(|ids| {
            request
                .asked
                .questions
                .iter()
                .any(|question| ids.contains(&question.unit))
        })
    });
}

/// Units whose planner already chose them by what the change did: an
/// outline or a document's outline by the members it added, a stale section
/// and a finished-plan question by the paths it removed, and a custom
/// question's unit by what the change did at its edges too, a hunk being
/// the change itself: one that only removes lines sits at the line after
/// them, outside what the change left.
fn chosen_when_planned(unit: &UnitPlan) -> bool {
    matches!(
        unit.detail,
        Detail::Outline { .. }
            | Detail::Document { .. }
            | Detail::Stale { .. }
            | Detail::Plan { .. }
            | Detail::Custom(_)
    )
}

/// A file whose syntax errors left no unit of any rule to judge is skipped
/// whole, as before partial parses: never reported empty and clear.
fn skip_left_out(scope: &Scope<'_>, result: &mut Plan) {
    for owner in &scope.owners {
        if scope.units[owner].partial()
            && result.files.get(owner).is_some_and(|f| f.units.is_empty())
        {
            result.files.remove(owner);
            result
                .skipped
                .insert(*owner, crate::syntax::SYNTAX_ERRORS.into());
        }
    }
}

/// Each GitHub Actions workflow file's jobs.
fn plan_workflows(scope: &Scope<'_>, args: &CheckArgs, budget: Limits<'_>, result: &mut Plan) {
    for &owner in &scope.configuration {
        let input = &scope.inputs[owner];
        if input.result.role != crate::inventory::WORKFLOW {
            continue;
        }
        let mut file = FilePlan {
            path: input.result.path.clone(),
            ..Default::default()
        };
        let source = input.source.as_deref().unwrap_or("");
        let context = FileContext::plain((owner, input), ("YAML", source), args, budget);
        workflows::plan(&context, &mut file, &mut result.requests);
        result.files.insert(owner, file);
    }
}

/// The documentation rules and custom questions for one agent instruction
/// file or project document.
fn plan_document(
    (owner, input): (usize, &Input),
    args: &CheckArgs,
    budget: Limits<'_>,
    drift: &drift::Shared<'_>,
    (requests, custom): (&mut Vec<Planned>, &mut custom::Planner),
) -> FilePlan {
    let since = requests.len();
    let mut file = FilePlan {
        path: input.result.path.clone(),
        ..Default::default()
    };
    // Other formats are read as Markdown with the file's own lines.
    let source =
        crate::docs::format::view(&input.result.path, input.source.as_deref().unwrap_or(""));
    let language = crate::docs::format::Format::of(&input.result.path).language();
    let context = FileContext::plain((owner, input), (language, &source), args, budget);
    if input.result.role == crate::inventory::DOCS {
        if args.enabled(catalog::LARGE_DOCS) {
            documents::plan(&context, &mut file, requests);
        }
    } else if let Some(repository) = &input.repository
        && args.enabled(catalog::AGENT_CONTEXT)
    {
        instructions::plan(&context, repository, &mut file, requests);
    }
    drift.plan(&context, &mut file, requests);
    custom.document(&context, &mut file, (requests, since));
    file
}

/// Parse every selected file and the explicit context. Files without a parser
/// or that the parser could not read are skipped with a reason, unless a
/// custom question that needs no parser reads a file without one; syntax
/// errors elsewhere leave out the units holding them, test cases included.
fn parsed_scope<'a>(
    inputs: &'a [Input],
    views: &'a BTreeMap<usize, View>,
    skipped: &mut BTreeMap<usize, String>,
    custom: &custom::Planner,
) -> Scope<'a> {
    let mut scope = Scope {
        owners: Vec::new(),
        inputs,
        views,
        units: BTreeMap::new(),
        context: Vec::new(),
        documents: Vec::new(),
        configuration: Vec::new(),
        texts: Vec::new(),
    };
    for (&owner, view) in views {
        let input = &inputs[owner];
        if view.classification.kind == crate::file_kind::TEXT {
            scope.texts.push((owner, false));
            continue;
        }
        if [crate::file_kind::INSTRUCTIONS, crate::file_kind::DOCS]
            .contains(&view.classification.kind.as_str())
        {
            scope.documents.push(owner);
            continue;
        }
        if view.classification.gate == "security" && !view.application {
            scope.configuration.push(owner);
            continue;
        }
        let source = input.source.as_deref().unwrap_or("");
        match parsed::parse(&input.result.path, source) {
            Ok(mut units) if units.parsed => {
                if units.partial()
                    && (view.tests || view.classification.kind == crate::file_kind::TESTS)
                {
                    leave_out_tests(input, &scope.test_lines(owner), &mut units);
                }
                scope.units.insert(owner, units);
                scope.owners.push(owner);
            }
            Ok(_) if custom.reads_text(&input.result.path) => scope.texts.push((owner, true)),
            Ok(_) => {
                let reason = format!(
                    "No {} parser; units cannot be located, so this file was not judged.",
                    view.classification.language
                );
                skipped.insert(owner, reason);
            }
            Err(error) => {
                skipped.insert(owner, crate::syntax::skip_reason(&error).into());
            }
        }
    }
    if let Some(first) = scope.owners.first() {
        for context in &inputs[*first].context {
            let units = parsed::parse(&context.file.path, &context.source).unwrap_or_default();
            scope
                .context
                .push((context.file.path.clone(), context.source.as_str(), units));
        }
    }
    scope
}

/// Leave out the test cases a partial parse broke, inside the file's test
/// `lines`, where the test rules and a test file's outline judge them.
fn leave_out_tests(input: &Input, lines: &[Range<usize>], units: &mut FileUnits) {
    let source = input.source.as_deref().unwrap_or("");
    let broken = crate::analysis::test_map::broken_cases(&input.result.path, source)
        .unwrap_or_default()
        .into_iter()
        .filter(|case| lines.iter().any(|l| l.contains(&case.line)))
        .collect();
    units.leave_out_tests(broken, source);
}
