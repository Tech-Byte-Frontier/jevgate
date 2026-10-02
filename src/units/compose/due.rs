//! Which follow-ups a file's recorded answers call for: the units each
//! follow-up stage asks next, in the order the stages run.
use super::*;

/// The settle Choices a security unit calls for, not yet asked: those whose
/// checks stay undecided after the trace and recheck while the unit is
/// uncertain, and for a consider resting on an undecided check, where its
/// text goes (the finding claims it likely reaches a client) and where code
/// that requests a URL runs; and those asked whenever their checks are not
/// clear, such as what a PHP page joins into HTML. A settle only clears, and
/// a note is not reported, so a note is not settled.
pub fn unsettled(unit: &UnitPlan, judgments: &[Judgment]) -> BTreeSet<&'static str> {
    use crate::units::security::{SETTLES, SettleWhen};
    if unit.presence != Presence::Judged
        || !security(unit.rule)
        || answers(judgments, &unit.id, Pass::Trace).is_empty()
    {
        return BTreeSet::new();
    }
    let merged = security_answers(unit, judgments);
    let outcome = unit_outcome(unit, &merged);
    if matches!(outcome, Outcome::Note(_)) {
        return BTreeSet::new();
    }
    let open = |when: SettleWhen| match outcome {
        Outcome::Uncertain(_) => true,
        Outcome::Consider(_) => when == SettleWhen::UndecidedOrFinding,
        _ => false,
    };
    let undecided = |q: &str| {
        merged
            .get(q)
            .is_some_and(|a| matches!(noul(a), Outcome::Uncertain(_)))
    };
    let not_clear = |q: &str| merged.get(q).is_some_and(|a| noul(a) != Outcome::Clear);
    SETTLES
        .iter()
        .filter(|kind| kind.rule == unit.rule && !merged.contains_key(kind.question))
        .filter(|kind| match kind.when {
            SettleWhen::NotClear => kind.checks.iter().any(|q| not_clear(q)),
            when => open(when) && kind.checks.iter().any(|q| undecided(q)),
        })
        .map(|kind| kind.question)
        .collect()
}

/// Security units whose finding a confirm Choice of their own follows, not
/// yet asked: an injection finding whose one concern is a path or markup
/// (what its paths or values can hold), or a redirect unless its values are
/// asked already (where it leads), and a sensitive-data finding its log
/// checks raised (when the log line runs). A markup consider on the
/// function's parameters is asked both what its values can hold and what
/// they hold where they enter the markup: escaped text another party wrote
/// is harmless there.
pub fn unconfirmed_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    confirm_due(plan, judgments, |u, outcome, resolved| {
        let Detail::Security { confirms, .. } = &u.detail else {
            return false;
        };
        let get = |q: &str| resolved.get(q).copied();
        let kind = confirmable(&get).filter(|k| !QUERIED.contains(k));
        confirms.checked.is_some()
            && kind.is_some_and(|k| k == "path" || k == "markup" || !values_due(outcome, resolved))
            || confirms.logging.is_some() && logs_found(&get)
    })
}

/// Injection findings whose one concern is SQL, a command or evaluated code,
/// not yet asked what their values hold where they enter it, unless what
/// their values can hold is asked already; weak-settings findings their
/// escaping check raised, not yet asked what the unescaped HTML holds; and
/// sensitive-data findings their error-detail checks raised, not yet asked
/// who reads the error text.
pub fn unqueried_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    confirm_due(plan, judgments, |u, outcome, resolved| {
        let Detail::Security { confirms, .. } = &u.detail else {
            return false;
        };
        let get = |q: &str| resolved.get(q).copied();
        confirms.queried.is_some()
            && confirmable(&get).is_some_and(|k| QUERIED.contains(&k))
            && !values_due(outcome, resolved)
            || confirms.rendered.is_some() && escape_found(&get)
            || confirms.readers.is_some() && errors_found(&get)
    })
}

/// Judged units with no locate answer yet whose outcome is a review or
/// consider and that `due` selects.
fn confirm_due(
    plan: &FilePlan,
    judgments: &[Judgment],
    due: impl Fn(&UnitPlan, Outcome, &Answers<'_>) -> bool,
) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged)
        .filter(|u| answers(judgments, &u.id, Pass::Locate).is_empty())
        .filter(|u| {
            let (outcome, resolved) = resolved(u, judgments);
            matches!(outcome, Outcome::Review(_) | Outcome::Consider(_))
                && due(u, outcome, &resolved)
        })
        .map(|u| u.id.clone())
        .collect()
}

/// Whether an injection outcome calls for what its values can hold: a
/// consider that rests on the function's parameters, its origin not
/// another party.
pub(super) fn values_due(outcome: Outcome, resolved: &Answers<'_>) -> bool {
    matches!(outcome, Outcome::Consider(_))
        && !resolved
            .get("origin")
            .is_some_and(|a| matches!(origin_outcome(a), Outcome::Review(_)))
}

/// Functions whose split question raised a review or consider, and whose
/// block has not been located yet; hardcoded-value functions raised to a
/// review or consider whose value has not been named yet; and the other
/// units whose confirm `locate_due` calls for.
pub fn unlocated_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged)
        .filter(|u| answers(judgments, &u.id, Pass::Locate).is_empty())
        .filter(|u| locate_due(u, judgments))
        .map(|u| u.id.clone())
        .collect()
}

/// Whether a unit's outcome calls for its locate or confirm follow-up.
pub(super) fn locate_due(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    let (outcome, resolved) = resolved(unit, judgments);
    let raised = |o: Option<Outcome>| matches!(o, Some(Outcome::Review(_) | Outcome::Consider(_)));
    match &unit.detail {
        Detail::TestPair {
            confirm: Some(_), ..
        } => matches!(outcome, Outcome::Review(_)),
        // Only the internal-details check raises a test's consider.
        Detail::Test { confirm: Some(_) } => matches!(outcome, Outcome::Consider(_)),
        // An injection consider rests on the function's parameters unless
        // its origin was another party; one that rests on a path is asked
        // what its paths can hold instead.
        Detail::Security { confirms, .. } if confirms.values.is_some() => {
            values_due(outcome, &resolved)
                && confirmable(&|q| resolved.get(q).copied()) != Some("path")
        }
        // Only a split review names a block: below it, the look-here
        // question decides, and its finding names no block.
        Detail::Function {
            locate: Some(_), ..
        } => matches!(
            resolved.get("split").map(|a| benefit(a)),
            Some(Outcome::Review(_))
        ),
        Detail::Document {
            locate: Some(_), ..
        } => raised(
            resolved
                .get("split")
                .map(|a| document_split(a, resolved.get("kind").copied())),
        ),
        _ => false,
    }
}

/// Judged units whose first pass stayed undecided, or became a note from a
/// torn function or file-organization answer, and that have no recheck yet;
/// for injection, units whose traced origin stayed unclear or was the
/// function's parameters, so callers can settle it.
pub fn uncertain_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged)
        .filter(|u| answers(judgments, &u.id, Pass::Recheck).is_empty())
        .filter(|u| {
            if security(u.rule) {
                origin_unsettled(u, judgments)
            } else {
                let first = answers(judgments, &u.id, Pass::First);
                open(u, &first, unit_outcome(u, &first))
            }
        })
        .map(|u| u.id.clone())
        .collect()
}

/// An injection unit whose checks are not all clear while its traced origin
/// stayed unclear or was the function's parameters.
pub(super) fn origin_unsettled(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    let merged = security_answers(unit, judgments);
    let get = |q: &str| merged.get(q).copied();
    unit.rule == catalog::INJECTION
        && !checks(unit.rule, &get).iter().all(|o| *o == Outcome::Clear)
        && matches!(
            merged.get("origin").map(|a| origin_outcome(a)),
            Some(Outcome::Uncertain(_) | Outcome::Consider(_))
        )
}

/// Outlines whose recheck left the split Score undecided, or whose first
/// answer did when the file is too long for a recheck, and large documents
/// whose split Score stayed undecided, whose kind has not been asked yet;
/// section pairs and stale sections whose checks stayed undecided and whose
/// settle has not been asked yet.
pub fn unkinded_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged)
        .filter(|u| match u.detail {
            Detail::DocPair { .. } | Detail::Stale { .. } => unsettled_check(u, judgments),
            Detail::Comment { .. } => unsettled_comment(u, judgments),
            _ => unkinded_split(u, judgments),
        })
        .map(|u| u.id.clone())
        .collect()
}

/// A large document not yet asked its kind whose split Score stayed
/// undecided or raised a finding, since its Score reads headings alone.
pub(super) fn unkinded_split(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    unit.rule == catalog::LARGE_DOCS
        && answers(judgments, &unit.id, Pass::Trace).is_empty()
        && answers(judgments, &unit.id, Pass::First)
            .get("split")
            .is_some_and(|a| {
                matches!(
                    benefit(a),
                    Outcome::Uncertain(_) | Outcome::Consider(_) | Outcome::Review(_)
                )
            })
}

/// A section pair or stale section whose checks were asked, stayed
/// undecided, and whose settle has not been asked.
pub(super) fn unsettled_check(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    !answers(judgments, &unit.id, Pass::Trace).is_empty()
        && answers(judgments, &unit.id, Pass::Settle).is_empty()
        && matches!(resolved(unit, judgments).0, Outcome::Uncertain(_))
}

/// A comment still undecided after its recheck, or without one, whose kind
/// has not been asked.
pub(super) fn unsettled_comment(unit: &UnitPlan, judgments: &[Judgment]) -> bool {
    answers(judgments, &unit.id, Pass::Settle).is_empty()
        && (unit.recheck.is_none() || !answers(judgments, &unit.id, Pass::Recheck).is_empty())
        && matches!(resolved(unit, judgments).0, Outcome::Uncertain(_))
}

/// Documents whose plan question found a plan whose work Git shows finished.
pub fn finished_plans(
    plan: &crate::units::Plan,
    files: &[crate::schema::FileResult],
) -> BTreeSet<std::path::PathBuf> {
    plan.files
        .iter()
        .filter(|(owner, file_plan)| {
            file_plan.units.iter().any(|u| {
                matches!(u.detail, Detail::Plan { .. })
                    && matches!(
                        resolved(u, &files[**owner].judgments).0,
                        Outcome::Consider(_) | Outcome::Review(_)
                    )
            })
        })
        .map(|(_, file_plan)| file_plan.path.clone())
        .collect()
}

/// Security units whose presence is not clear, to trace.
pub fn untraced_units(plan: &FilePlan, judgments: &[Judgment]) -> BTreeSet<String> {
    plan.units
        .iter()
        .filter(|u| u.presence == Presence::Judged && security(u.rule))
        .filter(|u| answers(judgments, &u.id, Pass::Trace).is_empty())
        .filter(|u| {
            let first = answers(judgments, &u.id, Pass::First);
            let presence: Vec<Outcome> = crate::units::security::presence_questions(u.rule)
                .iter()
                .filter_map(|q| first.get(q).map(|a| noul(a)))
                .collect();
            if presence.is_empty() {
                return false;
            }
            presence.iter().any(|o| *o != Outcome::Clear)
        })
        .map(|u| u.id.clone())
        .collect()
}
