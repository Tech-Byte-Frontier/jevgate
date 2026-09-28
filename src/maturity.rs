//! Which rules and levels fail the gate by default. A rule and level is
//! mature when its findings were right at least 80% of the time on projects
//! JevGate was never tuned on, over at least 20 findings labeled from the
//! code. The default gate level, `mature`, fails only on those; every
//! other finding is reported without failing the check, until its rule and
//! level measure up. The concern probability could not decide this: on those
//! projects, reviews were right 55%, 46%, 56% and 61% of the time with a
//! probability below 0.90, below 0.95, below 0.98 and above.
use crate::{
    catalog,
    schema::Strength::{self, Consider, Review},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Labeled findings a rule and level needs on unseen projects to be mature.
pub const MIN_LABELS: u32 = 20;
/// The share of those findings, in percent, that must be right.
pub const MIN_PERCENT_RIGHT: u32 = 80;

/// Findings of one rule and level labeled from the code: how many were
/// right, of how many labeled. A debatable label counts as not right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labels {
    pub right: u32,
    pub labeled: u32,
}

impl Labels {
    /// The share right in whole percent, half rounded up as the HTML report
    /// and the site round it; none without labels.
    pub fn percent(self) -> Option<u32> {
        (self.labeled > 0).then(|| (200 * self.right + self.labeled) / (2 * self.labeled))
    }

    /// "87% of 23"; none without labels.
    pub fn summary(self) -> Option<String> {
        self.percent().map(|p| format!("{p}% of {}", self.labeled))
    }

    /// How often such findings were right, for a reader: "right 87% of the
    /// time (23 labels)", or "not yet measured" below [`MIN_LABELS`], where a
    /// share says little.
    pub fn in_words(self) -> String {
        match self.percent() {
            Some(p) if self.labeled >= MIN_LABELS => {
                format!("right {p}% of the time ({} labels)", self.labeled)
            }
            _ => "not yet measured".into(),
        }
    }
}

/// One rule and level's labels on the projects never used for tuning and on
/// the ones the rules were tuned on.
pub struct Measure {
    /// The rule's catalog key.
    pub rule: &'static str,
    pub level: Strength,
    pub unseen: Labels,
    pub tuned: Labels,
}

impl Measure {
    /// Right at least [`MIN_PERCENT_RIGHT`] of the time over at least
    /// [`MIN_LABELS`] labels on unseen projects.
    pub fn mature(&self) -> bool {
        let Labels { right, labeled } = self.unseen;
        labeled >= MIN_LABELS && 100 * right >= MIN_PERCENT_RIGHT * labeled
    }
}

const fn row(rule: &'static str, level: Strength, unseen: [u32; 2], tuned: [u32; 2]) -> Measure {
    Measure {
        rule,
        level,
        unseen: Labels {
            right: unseen[0],
            labeled: unseen[1],
        },
        tuned: Labels {
            right: tuned[0],
            labeled: tuned[1],
        },
    }
}

/// Measured on 2026-09-28 from the corpus's labels
/// (`evaluation/labels`, joined by fingerprint) and JevGate's findings with the
/// shared-logic consider threshold of `policy::CALIBRATED`, replayed from the
/// answer cache over the 94 labeled projects outside Bend 2; the few files
/// whose requests the cache lacked keep their 0.24.1 findings. Unseen:
/// [right, labeled] on the 11 held-out and 14 fresh projects never used for
/// tuning (22 of them have findings); tuned: on the other 72.
/// `docs/research/2026-09-28/scripts/maturity.py` in the maintainer's clone
/// prints these rows. Two are mature: function-simplification reviews (20 of
/// 23) and agent-context considers (22 of 24). The gap between the columns is
/// why only unseen projects count: shared-logic reviews were right 75% of the
/// time on tuned projects and 54% on unseen ones.
const TABLE: [Measure; 26] = [
    row(catalog::FILE_ORGANIZATION, Review, [2, 5], [15, 30]),
    row(catalog::FILE_ORGANIZATION, Consider, [17, 29], [19, 43]),
    row(catalog::FUNCTION_SIMPLIFICATION, Review, [20, 23], [57, 69]),
    row(
        catalog::FUNCTION_SIMPLIFICATION,
        Consider,
        [85, 126],
        [147, 197],
    ),
    row(catalog::SHARED_LOGIC, Review, [46, 85], [181, 240]),
    row(catalog::SHARED_LOGIC, Consider, [76, 129], [121, 190]),
    row(catalog::HARDCODED_VALUES, Review, [1, 8], [15, 28]),
    row(catalog::HARDCODED_VALUES, Consider, [5, 29], [32, 57]),
    row(catalog::INJECTION, Review, [3, 4], [81, 96]),
    row(catalog::INJECTION, Consider, [5, 13], [27, 47]),
    row(catalog::SENSITIVE_DATA, Review, [10, 24], [41, 64]),
    row(catalog::SENSITIVE_DATA, Consider, [0, 5], [12, 15]),
    row(catalog::UNSAFE_SETTINGS, Review, [2, 4], [53, 72]),
    row(catalog::UNSAFE_SETTINGS, Consider, [0, 4], [16, 21]),
    row(catalog::ACCESS_CONTROL, Review, [0, 0], [2, 5]),
    row(catalog::ACCESS_CONTROL, Consider, [0, 0], [5, 14]),
    row(catalog::WORKFLOWS, Review, [1, 1], [0, 1]),
    row(catalog::TEST_VALUE, Review, [3, 5], [5, 11]),
    row(catalog::TEST_VALUE, Consider, [1, 2], [20, 27]),
    row(catalog::TEST_REDUNDANCY, Review, [1, 1], [4, 4]),
    row(catalog::TEST_REDUNDANCY, Consider, [27, 45], [27, 34]),
    row(catalog::AGENT_CONTEXT, Consider, [22, 24], [64, 68]),
    row(catalog::LARGE_DOCS, Consider, [1, 1], [1, 5]),
    row(catalog::DOC_STALENESS, Consider, [2, 2], [14, 15]),
    row(catalog::DOC_DUPLICATION, Consider, [3, 20], [5, 22]),
    row(catalog::COMMENTS, Consider, [39, 72], [97, 150]),
];

/// The labels of a rule, by ID, name or key, at one level.
pub fn measure(rule: &str, level: Strength) -> Option<&'static Measure> {
    let key = catalog::find(rule)?.key;
    TABLE.iter().find(|m| m.rule == key && m.level == level)
}

/// Why the laws rule has no share: it judges only Bend 2 code, whose labeled
/// projects this table leaves out.
const BEND_2_ONLY: &str = "labeled only on Bend 2 projects, which the maturity table leaves out";

fn bend_2_only(rule: &str) -> bool {
    catalog::find(rule).is_some_and(|found| found.key == catalog::LAWS)
}

/// Why `rule` has no share of right findings on unseen projects at a level:
/// the laws rule judges Bend 2 code, whose labeled projects this table
/// leaves out; any other has no labeled finding there yet.
pub fn unmeasured(rule: &str) -> &'static str {
    if bend_2_only(rule) {
        BEND_2_ONLY
    } else {
        "none labeled yet on projects JevGate was never tuned on"
    }
}

/// How often findings of `rule` with `labels` were right, for a reader:
/// [`Labels::in_words`], and for a law finding, which has none, that its
/// labels are Bend 2's: "not yet measured" alone would say none were made.
pub fn precision_in_words(rule: &str, labels: Labels) -> String {
    let words = labels.in_words();
    if labels.labeled == 0 && bend_2_only(rule) {
        format!("{words}: {BEND_2_ONLY}")
    } else {
        words
    }
}

/// The labels a finding of `rule` at `level` carries: its rule and level's on
/// unseen projects, none labeled when the table has no row for them, and
/// none for a note, which is never labeled.
pub fn precision(rule: &str, level: Strength) -> Option<Labels> {
    (level != Strength::Note)
        .then(|| measure(rule, level).map_or_else(Labels::default, |m| m.unseen))
}

/// Whether the default gate fails on findings of `rule` at `level`.
pub fn mature(rule: &str, level: Strength) -> bool {
    measure(rule, level).is_some_and(Measure::mature)
}

/// A rule's mature levels, review first.
pub fn mature_levels(rule: &str) -> Vec<Strength> {
    [Review, Consider]
        .into_iter()
        .filter(|level| mature(rule, *level))
        .collect()
}

/// A rule's measured levels for `jevgate rules --format json`: labels on
/// unseen and tuned projects, and whether each level is mature.
pub fn describe(rule: &str) -> Value {
    let levels = [Review, Consider].into_iter().filter_map(|level| {
        measure(rule, level).map(|m| {
            let value = json!({
                "unseen": m.unseen,
                "tuned": m.tuned,
                "mature": m.mature(),
            });
            (crate::output::label(&level), value)
        })
    });
    Value::Object(levels.collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unseen(right: u32, labeled: u32) -> Measure {
        row(catalog::SHARED_LOGIC, Review, [right, labeled], [0, 0])
    }

    #[test]
    fn a_level_is_mature_at_eighty_percent_over_twenty_labels() {
        assert!(unseen(16, 20).mature(), "exactly 80% of 20");
        assert!(!unseen(15, 20).mature());
        assert!(!unseen(19, 19).mature(), "too few labels");
        assert!(unseen(20, 23).mature());
        assert!(!unseen(0, 0).mature());
    }

    #[test]
    fn the_table_names_catalog_rules_once_per_level() {
        let keys = catalog::keys();
        for (i, m) in TABLE.iter().enumerate() {
            assert!(keys.contains(&m.rule), "{}", m.rule);
            assert!(m.level != Strength::Note, "{}", m.rule);
            for labels in [m.unseen, m.tuned] {
                assert!(labels.right <= labels.labeled, "{}", m.rule);
            }
            assert!(
                !TABLE[..i]
                    .iter()
                    .any(|other| other.rule == m.rule && other.level == m.level),
                "{} twice",
                m.rule
            );
        }
    }

    #[test]
    fn function_simplification_reviews_and_agent_context_considers_are_mature() {
        let mature: Vec<(&str, Strength)> = TABLE
            .iter()
            .filter(|m| m.mature())
            .map(|m| (m.rule, m.level))
            .collect();
        assert_eq!(
            mature,
            [
                (catalog::FUNCTION_SIMPLIFICATION, Review),
                (catalog::AGENT_CONTEXT, Consider)
            ]
        );
        assert!(self::mature(
            "maintainability/function-simplification",
            Review
        ));
        assert!(!self::mature("function-simplification", Consider));
        assert!(!self::mature(catalog::LAWS, Review), "never measured");
        assert_eq!(mature_levels(catalog::AGENT_CONTEXT), [Consider]);
        assert!(mature_levels("nothing").is_empty());
    }

    #[test]
    fn measured_levels_describe_their_labels() {
        let value = describe(catalog::FUNCTION_SIMPLIFICATION);
        assert_eq!(
            value["review"],
            json!({"unseen": {"right": 20, "labeled": 23}, "tuned": {"right": 57, "labeled": 69}, "mature": true})
        );
        assert_eq!(value["consider"]["mature"], false);
        assert_eq!(describe(catalog::LAWS), json!({}));
        let unseen = |rule| measure(rule, Review).unwrap().unseen.summary();
        assert_eq!(unseen(catalog::SHARED_LOGIC).as_deref(), Some("54% of 85"));
        assert_eq!(
            unseen(catalog::HARDCODED_VALUES).as_deref(),
            Some("13% of 8"),
            "12.5% rounds up"
        );
        assert_eq!(unseen(catalog::ACCESS_CONTROL), None);
    }

    #[test]
    fn a_level_without_a_share_says_why() {
        assert_eq!(
            unmeasured(catalog::ACCESS_CONTROL),
            "none labeled yet on projects JevGate was never tuned on"
        );
        assert_eq!(
            unmeasured("tests/laws"),
            "labeled only on Bend 2 projects, which the maturity table leaves out",
            "law findings were labeled, on the Bend 2 projects kept apart"
        );
    }

    #[test]
    fn a_finding_carries_its_levels_labels_and_a_reader_sees_them_from_twenty() {
        let labels = |rule, level| precision(rule, level).unwrap();
        assert_eq!(
            labels("maintainability/function-simplification", Review).in_words(),
            "right 87% of the time (23 labels)"
        );
        let shared = labels(catalog::SHARED_LOGIC, Consider);
        assert_eq!(shared, unseen(76, 129).unseen);
        assert_eq!(shared.in_words(), "right 59% of the time (129 labels)");
        assert_eq!(unseen(19, 19).unseen.in_words(), "not yet measured");
        assert_eq!(
            unseen(16, 20).unseen.in_words(),
            "right 80% of the time (20 labels)"
        );
        let never = labels(catalog::LAWS, Review);
        assert_eq!(never, Labels::default(), "no row: none labeled");
        assert_eq!(never.in_words(), "not yet measured");
        assert_eq!(
            precision_in_words(catalog::LAWS, never),
            "not yet measured: labeled only on Bend 2 projects, which the maturity table leaves out"
        );
        let none = labels(catalog::ACCESS_CONTROL, Consider);
        assert_eq!(
            precision_in_words(catalog::ACCESS_CONTROL, none),
            "not yet measured"
        );
        assert_eq!(precision(catalog::SHARED_LOGIC, Strength::Note), None);
    }
}
