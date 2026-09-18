use crate::schema::{Change, FileResult, Report, Status};

fn equivalent(old: &FileResult, new: &FileResult, previous: &Report, current: &Report) -> bool {
    previous.rubric_version == current.rubric_version
        && previous.requested_model == current.requested_model
        && old.model == new.model
        && old.role == new.role
        && old.context_files == new.context_files
        && old.context_complete
        && new.context_complete
        && !matches!(old.status, Status::Error | Status::Pending)
        && !matches!(new.status, Status::Error | Status::Pending)
}

pub fn compare(previous: Option<&Report>, report: &mut Report) {
    report.changes.clear();
    let Some(previous) = previous else {
        for file in &report.files {
            for finding in &file.findings {
                report.changes.push(Change {
                    rule: finding.rule.clone(),
                    path: file.path.clone(),
                    previous_path: None,
                    previous_generation: None,
                    state: "baseline".into(),
                    reason: "First observed assessment; introduction time is unknown".into(),
                });
            }
        }
        return;
    };
    let mut matched = std::collections::BTreeSet::new();
    for file in &report.files {
        let exact = previous.files.iter().find(|old| old.path == file.path);
        let candidates: Vec<_> = previous
            .files
            .iter()
            .filter(|old| {
                !old.content_identity.is_empty()
                    && old.content_identity == file.content_identity
                    && !report.files.iter().any(|f| f.path == old.path)
            })
            .collect();
        let old = exact.or_else(|| {
            (candidates.len() == 1
                && report
                    .files
                    .iter()
                    .filter(|f| f.content_identity == file.content_identity)
                    .count()
                    == 1)
                .then(|| candidates[0])
        });
        if let Some(old) = old {
            matched.insert(old.path.clone());
        }
        for rule in crate::catalog::rules() {
            let before = old.and_then(|f| f.dimensions.get(rule.key));
            let after = file.dimensions.get(rule.key);
            if before.is_none_or(|d| d.status != Status::Review)
                && after.is_none_or(|d| d.status != Status::Review)
            {
                continue;
            }
            let comparable = old.is_some_and(|old| equivalent(old, file, previous, report));
            let before_review = before.is_some_and(|d| d.status == Status::Review);
            let after_review = after.is_some_and(|d| d.status == Status::Review);
            let (state, reason) = if !comparable || before.is_none() || after.is_none() {
                (
                    "non-comparable",
                    "Scope, context, rule/model identity or execution changed",
                )
            } else if before_review && after_review {
                ("persistent", "Concern remains under comparable evidence")
            } else if before_review && after.is_some_and(|d| d.status == Status::Clear) {
                // Changed source can remove or move behavior; do not credit it as a fix without unit lineage.
                if old.unwrap().content_identity == file.content_identity
                    || (old.unwrap().syntax_checked
                        && file.syntax_checked
                        && !file.symbols.is_empty()
                        && old.unwrap().symbols == file.symbols
                        && file.semantic_size >= old.unwrap().semantic_size)
                {
                    (
                        "resolved",
                        "Assessment no longer triggers with preserved symbols and comparable evidence; correctness is not certified",
                    )
                } else {
                    (
                        "non-comparable",
                        "Behavior changed; file-level review cannot prove scope was preserved or follow helper extraction",
                    )
                }
            } else if after_review && before.is_some_and(|d| d.status == Status::Clear) {
                ("introduced", "Previously clear scope now triggers a review")
            } else {
                ("uncertain", "One assessment abstains")
            };
            report.changes.push(Change {
                rule: rule.id.into(),
                path: file.path.clone(),
                previous_path: old.map(|f| f.path.clone()),
                previous_generation: Some(previous.generation),
                state: state.into(),
                reason: reason.into(),
            });
        }
    }
    for old in previous.files.iter().filter(|f| !matched.contains(&f.path)) {
        for finding in &old.findings {
            report.changes.push(Change {
                rule: finding.rule.clone(),
                path: old.path.clone(),
                previous_path: Some(old.path.clone()),
                previous_generation: Some(previous.generation),
                state: "non-comparable".into(),
                reason: "Deleted, moved ambiguously or excluded scope is not a verified resolution"
                    .into(),
            });
        }
    }
}
