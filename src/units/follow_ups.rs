//! Follow-up requests that recorded answers call for: traces of security
//! units, rechecks of undecided units and locating split findings.
use super::{Detail, Plan, Planned, UnitPlan, compose};
use crate::schema::{FileResult, Judgment, Status};
use serde_json::Value;
use std::collections::BTreeSet;

/// One locate follow-up per function whose split raised a review or consider.
pub fn locates(plan: &Plan, files: &[FileResult]) -> Vec<Planned> {
    follow_ups(plan, files, compose::unlocated_units, |unit| {
        match &unit.detail {
            Detail::Function { locate, .. } => locate.as_ref(),
            _ => None,
        }
    })
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

/// The planned follow-up of every selected unit, skipping failed files.
fn follow_ups(
    plan: &Plan,
    files: &[FileResult],
    select: fn(&super::FilePlan, &[Judgment]) -> BTreeSet<String>,
    follow_up: fn(&UnitPlan) -> Option<&(Value, super::Asked)>,
) -> Vec<Planned> {
    let mut planned = Vec::new();
    for (&owner, file_plan) in &plan.files {
        let file = &files[owner];
        if file.status == Status::Error {
            continue;
        }
        let selected = select(file_plan, &file.judgments);
        for unit in &file_plan.units {
            if let Some((request, asked)) = follow_up(unit)
                && selected.contains(&unit.id)
            {
                planned.push(Planned {
                    owner,
                    request: request.clone(),
                    asked: asked.clone(),
                });
            }
        }
    }
    planned
}
