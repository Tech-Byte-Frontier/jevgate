//! Comment findings: a unit's comments to clean up, reported together.
use super::*;

pub(super) fn documented(unit: &UnitPlan) -> bool {
    matches!(
        unit.detail,
        Detail::Comment {
            documentation: true,
            ..
        }
    )
}

/// Lines a unit's comments may span in all and still be few: a comment or
/// two a reader skips in a moment cost little.
pub(super) const FEW_COMMENT_LINES: usize = 3;

/// Comments raised to a consider whose unit's considered comments span
/// fewer than `FEW_COMMENT_LINES` lines in all: their finding is a note.
pub(super) fn few_comment_lines<'a>(
    plan: &'a FilePlan,
    judgments: &[Judgment],
) -> BTreeSet<&'a str> {
    let mut considered = BTreeMap::<&str, Vec<&UnitPlan>>::new();
    for unit in &plan.units {
        if let Detail::Comment { owner, .. } = &unit.detail
            && unit.presence == Presence::Judged
            && matches!(resolved(unit, judgments).0, Outcome::Consider(_))
        {
            considered.entry(owner.as_str()).or_default().push(unit);
        }
    }
    considered
        .into_values()
        .filter(|units| units.iter().map(|u| u.lines).sum::<usize>() < FEW_COMMENT_LINES)
        .flatten()
        .map(|u| u.id.as_str())
        .collect()
}

/// One finding per unit and strength for its comments a reader could do
/// without, listing each with what makes it so, at the lowest probability
/// among them.
pub(super) fn comment_findings(
    plan: &FilePlan,
    commented: &[(&UnitPlan, Strength, f64, &'static str)],
) -> Vec<Finding> {
    let mut grouped =
        BTreeMap::<(&str, Strength), Vec<&(&UnitPlan, Strength, f64, &'static str)>>::new();
    for entry in commented {
        let Detail::Comment { owner, .. } = &entry.0.detail else {
            continue;
        };
        grouped
            .entry((owner.as_str(), entry.1))
            .or_default()
            .push(entry);
    }
    grouped
        .into_iter()
        .map(|((owner, strength), mut entries)| {
            entries.sort_by_key(|(unit, ..)| unit.locations[0].start_line);
            let p = entries.iter().map(|(_, _, p, _)| *p).fold(1.0, f64::min);
            let listed: Vec<(&crate::schema::Location, &'static str)> = entries
                .iter()
                .map(|(unit, _, _, reason)| (&unit.locations[0], *reason))
                .collect();
            let (message, action) = comment_wording(owner, &listed, strength);
            let locations: Vec<crate::schema::Location> =
                listed.iter().map(|(l, _)| (*l).clone()).collect();
            let lines = entries.iter().map(|(unit, ..)| unit.lines).sum();
            let identities: Vec<&str> = std::iter::once(owner)
                .chain(entries.iter().map(|(unit, ..)| unit.identity.as_str()))
                .collect();
            Finding {
                rule: catalog::id(catalog::COMMENTS).into(),
                strength,
                measured_as: Some(strength),
                line: locations[0].start_line,
                message,
                action: action.into(),
                symbol: (owner != crate::units::comments::TOP_LEVEL).then(|| owner.to_string()),
                rule_version: catalog::rule_version(catalog::COMMENTS).into(),
                concern_probability: p,
                locations,
                quote: entries[0].0.quote.clone(),
                category: None,
                fingerprint: fingerprint(
                    catalog::COMMENTS,
                    plan,
                    &crate::units::identity(&identities),
                ),
                rank: rank(p, lines),
                baselined: false,
                suppressed: None,
                gate: None,
                precision: None,
                preview: None,
                untouched: Vec::new(),
            }
        })
        .collect()
}
