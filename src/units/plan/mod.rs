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
}

use super::{
    FileContext, FilePlan, Plan, Planned, access, documents, drift, handlers, instructions,
    workflows,
};

use crate::{
    analysis::units::{self as parsed, FileUnits, Unit},
    catalog,
    file_kind::View,
    inventory::Input,
    options::CheckArgs,
    token_budget::TokenBudget,
};

use std::{
    collections::BTreeMap,
    ops::Range,
    path::{Path, PathBuf},
};

mod file;
mod security_units;
mod settings_modules;
mod shared;

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
}

pub fn plan(
    inputs: &[Input],
    views: &BTreeMap<usize, View>,
    args: &CheckArgs,
    budget: &TokenBudget,
) -> Plan {
    let mut result = Plan::default();
    let scope = parsed_scope(inputs, views, &mut result.skipped);
    let shared = Shared::new(&scope, args);
    for &owner in &scope.owners {
        let file = plan_file(&scope, &shared, owner, args, budget, &mut result.requests);
        result.files.insert(owner, file);
    }
    if shared.enabled(catalog::SENSITIVE_DATA) {
        let evidence = handlers::Evidence {
            imports: &shared.imports,
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
    for &owner in &scope.documents {
        let file = plan_document(
            &inputs[owner],
            owner,
            args,
            budget,
            &drift,
            &mut result.requests,
        );
        result.files.insert(owner, file);
    }
    result
}

/// Each GitHub Actions workflow file's jobs.
fn plan_workflows(scope: &Scope<'_>, args: &CheckArgs, budget: &TokenBudget, result: &mut Plan) {
    for &owner in &scope.configuration {
        let input = &scope.inputs[owner];
        if input.result.role != crate::inventory::WORKFLOW {
            continue;
        }
        let mut file = FilePlan {
            path: input.result.path.clone(),
            ..Default::default()
        };
        let context = FileContext {
            owner,
            path: &input.result.path,
            language: "YAML",
            source: input.source.as_deref().unwrap_or(""),
            source_hash: &input.result.source_hash,
            model: args.model(),
            budget,
            framework: None,
        };
        workflows::plan(&context, &mut file, &mut result.requests);
        result.files.insert(owner, file);
    }
}

/// The documentation rules for one agent instruction file.
fn plan_document(
    input: &Input,
    owner: usize,
    args: &CheckArgs,
    budget: &TokenBudget,
    drift: &drift::Shared<'_>,
    requests: &mut Vec<Planned>,
) -> FilePlan {
    let mut file = FilePlan {
        path: input.result.path.clone(),
        ..Default::default()
    };
    // Other formats are read as Markdown with the file's own lines.
    let source =
        crate::docs::format::view(&input.result.path, input.source.as_deref().unwrap_or(""));
    let context = FileContext {
        owner,
        path: &input.result.path,
        language: crate::docs::format::Format::of(&input.result.path).language(),
        source: &source,
        source_hash: &input.result.source_hash,
        model: args.model(),
        budget,
        framework: None,
    };
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
    file
}

/// Parse every selected file and the explicit context. Files without a parser
/// or with syntax errors are skipped with a reason.
fn parsed_scope<'a>(
    inputs: &'a [Input],
    views: &'a BTreeMap<usize, View>,
    skipped: &mut BTreeMap<usize, String>,
) -> Scope<'a> {
    let mut scope = Scope {
        owners: Vec::new(),
        inputs,
        views,
        units: BTreeMap::new(),
        context: Vec::new(),
        documents: Vec::new(),
        configuration: Vec::new(),
    };
    for (&owner, view) in views {
        let input = &inputs[owner];
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
            Ok(units) if units.parsed => {
                scope.units.insert(owner, units);
                scope.owners.push(owner);
            }
            Ok(_) => {
                let reason = format!(
                    "No {} parser; units cannot be located, so this file was not judged.",
                    view.classification.language
                );
                skipped.insert(owner, reason);
            }
            Err(_) => {
                skipped.insert(owner, "Syntax errors; this file was not judged.".into());
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
