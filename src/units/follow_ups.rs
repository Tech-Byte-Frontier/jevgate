//! Follow-up requests that recorded answers call for: traces of security
//! units, rechecks of undecided units, the kind of an outline still undecided,
//! the parts of a long file without a finding and locating split findings.
use super::{Detail, FollowUp, Plan, Planned, UnitPlan, compose};
use crate::schema::{FileResult, Judgment, Status};
use std::collections::BTreeSet;

/// One locate follow-up per function whose split raised a review or consider,
/// per hardcoded-value function raised to a review or consider, per redundant
/// test pair raised to a review, per test that asserts internal details, per
/// injection consider that rests on its parameters, per injection finding
/// that rests on a path, markup, a redirect, SQL, a command or code, per
/// weak-settings finding that rests on unescaped HTML and per logging
/// finding.
pub fn locates(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    let mut planned = follow_ups(
        plan,
        files,
        compose::unconfirmed_units,
        |unit| match &unit.detail {
            Detail::Security { confirms, .. } => {
                confirms.checked.as_ref().or(confirms.logging.as_ref())
            }
            _ => None,
        },
    );
    planned.extend(follow_ups(
        plan,
        files,
        compose::unqueried_units,
        |unit| match &unit.detail {
            Detail::Security { confirms, .. } => confirms
                .queried
                .as_ref()
                .or(confirms.rendered.as_ref())
                .or(confirms.readers.as_ref()),
            _ => None,
        },
    ));
    planned.extend(follow_ups(
        plan,
        files,
        compose::unlocated_units,
        |unit| match &unit.detail {
            Detail::Function { locate, .. }
            | Detail::Document { locate, .. }
            | Detail::Values { locate, .. }
            | Detail::Constants { locate, .. } => locate.as_ref(),
            Detail::TestPair { confirm, .. } | Detail::Test { confirm } => confirm.as_ref(),
            Detail::Security { confirms, .. } => confirms.values.as_ref(),
            _ => None,
        },
    ));
    planned
}

/// One question per hardcoded-value consider resting on a value's name
/// whose value its file writes again: what that value is. It needs the
/// value the locate named, so it follows the locates.
pub fn value_kinds(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    let mut planned = Vec::new();
    for (&owner, file_plan) in &plan.files {
        let file = &files[owner];
        if file.status == Status::Error {
            continue;
        }
        for (request, asked) in compose::unkinded_values(file_plan, &file.judgments) {
            planned.push(Planned {
                owner,
                request,
                asked,
            });
        }
    }
    planned
}

/// The section and pair checks of documents that are not finished plans.
pub fn doc_checks(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    let finished = compose::finished_plans(plan, files);
    let mut planned = Vec::new();
    for (&owner, file_plan) in &plan.files {
        let file = &files[owner];
        if file.status == Status::Error || finished.contains(&file_plan.path) {
            continue;
        }
        for unit in &file_plan.units {
            let (check, other) = match &unit.detail {
                Detail::Stale { check, .. } => (check, None),
                Detail::DocPair { check, other, .. } => (check, Some(&other.path)),
                _ => continue,
            };
            let asked = file
                .judgments
                .iter()
                .any(|j| j.unit == unit.id && j.pass == crate::schema::Pass::Trace);
            if let Some(check) = check
                && !asked
                && other.is_none_or(|p| !finished.contains(p))
            {
                planned.push(check.planned(owner));
            }
        }
    }
    planned
}

/// One trace per security unit whose presence answers call for it.
pub fn traces(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    follow_ups(plan, files, compose::untraced_units, |unit| {
        match &unit.detail {
            Detail::Security { trace, .. } => trace.as_ref(),
            _ => None,
        }
    })
}

/// One recheck per unit that stayed uncertain after the first pass.
pub fn rechecks(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    follow_ups(plan, files, compose::uncertain_units, |unit| {
        unit.recheck.as_ref()
    })
}

/// The settle follow-ups of each security unit whose checks stayed
/// undecided after its trace and recheck, one per kind of check.
pub fn settles(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    let mut planned = Vec::new();
    for (&owner, file_plan) in &plan.files {
        let file = &files[owner];
        if file.status == Status::Error {
            continue;
        }
        for unit in &file_plan.units {
            let Detail::Security { settles, .. } = &unit.detail else {
                continue;
            };
            let open = compose::unsettled(unit, &file.judgments);
            for settle in settles.iter().filter(|s| open.contains(s.question)) {
                planned.push(settle.request.planned(owner));
            }
        }
    }
    planned
}

/// One kind question per outline whose recheck stayed undecided, per large
/// document whose split stayed undecided, per section pair or stale section
/// whose checks stayed undecided, and per comment still undecided.
pub fn kinds(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    follow_ups(plan, files, compose::unkinded_units, |unit| {
        match &unit.detail {
            Detail::Outline { kind, .. } | Detail::Document { kind, .. } => kind.as_ref(),
            Detail::DocPair { settle, .. } | Detail::Stale { settle, .. } => settle.as_ref(),
            Detail::Comment { kind, .. } => kind.as_ref(),
            _ => None,
        }
    })
}

/// Whether each candidate part of a long file does a job of its own, asked
/// once the file's outline, its recheck and its kind raised no finding.
pub fn parts(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    let mut planned = Vec::new();
    for (&owner, file_plan) in &plan.files {
        let file = &files[owner];
        if file.status == Status::Error {
            continue;
        }
        let selected = compose::unparted_units(file_plan, &file.judgments);
        for unit in &file_plan.units {
            if let Detail::Outline { parts, .. } = &unit.detail
                && selected.contains(&unit.id)
            {
                planned.extend(parts.iter().map(|part| part.follow_up.planned(owner)));
            }
        }
    }
    planned
}

/// The planned follow-up of every selected unit, skipping failed files.
fn follow_ups(
    plan: &Plan,
    files: &[FileResult],
    select: fn(&super::FilePlan, &[Judgment]) -> BTreeSet<String>,
    follow_up: fn(&UnitPlan) -> Option<&FollowUp>,
) -> Vec<Planned> {
    let mut planned = Vec::new();
    for (&owner, file_plan) in &plan.files {
        let file = &files[owner];
        if file.status == Status::Error {
            continue;
        }
        let selected = select(file_plan, &file.judgments);
        for unit in &file_plan.units {
            if let Some(follow_up) = follow_up(unit)
                && selected.contains(&unit.id)
            {
                planned.push(follow_up.planned(owner));
            }
        }
    }
    planned
}
