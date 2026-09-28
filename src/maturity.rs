//! Which rules and levels fail the gate by default. A rule and level is
//! mature when its findings were right at least 80% of the time on projects
//! JevGate was never tuned on, over at least 20 findings labeled from the
//! code. The default gate level, `mature`, fails only on those; every
//! other finding is reported without failing the check, until its rule and
//! level measure up. The concern probability could not decide this: on those
//! projects, reviews were right 55%, 46%, 56% and 61% of the time with a
//! probability below 0.90, below 0.95, below 0.98 and above.
use crate::{
    catalog::{self, COMMENTS, FILE_ORGANIZATION, FUNCTION_SIMPLIFICATION, SHARED_LOGIC},
    schema::Strength::{self, Consider, Review},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

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

    /// "87% of 23", or below [`MIN_LABELS`], where a finding says "not yet
    /// measured", the counts: "2 of 5", as the site gives them; none without
    /// labels.
    pub fn summary(self) -> Option<String> {
        let p = self.percent()?;
        Some(if self.labeled >= MIN_LABELS {
            format!("{p}% of {}", self.labeled)
        } else {
            format!("{} of {}", self.right, self.labeled)
        })
    }

    /// How often such findings were right, for a reader: "right 87% of the
    /// time (23 labels)", or "not yet measured" below [`MIN_LABELS`], where a
    /// share says little.
    pub fn in_words(self) -> String {
        self.words("")
    }

    /// [`Labels::in_words`] for one language's own labels: "right 73% of
    /// the time in Swift (30 labels)", or "not yet measured in Kotlin".
    pub fn in_words_in(self, language: &str) -> String {
        self.words(&format!(" in {language}"))
    }

    fn words(self, place: &str) -> String {
        match self.percent() {
            Some(p) if self.labeled >= MIN_LABELS => {
                format!("right {p}% of the time{place} ({} labels)", self.labeled)
            }
            _ => format!("not yet measured{place}"),
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

/// One preview language's labels at a rule and level, on projects never
/// used for tuning.
struct PreviewMeasure {
    language: &'static str,
    /// The rule's catalog key.
    rule: &'static str,
    level: Strength,
    unseen: Labels,
}

const fn preview_row(
    language: &'static str,
    rule: &'static str,
    level: Strength,
    unseen: [u32; 2],
) -> PreviewMeasure {
    PreviewMeasure {
        language,
        rule,
        level,
        unseen: Labels {
            right: unseen[0],
            labeled: unseen[1],
        },
    }
}

/// The preview languages' labels, measured on 2026-09-28 by 0.30's first run
/// of the four rules they get on 37 well-known projects chosen for them and
/// never used for tuning (3 to 8 a language), all 598 reviews and considers
/// labeled by hand from the code, a debatable one counting as not right
/// (`evaluation/labels/parts/0.30-*.jsonl` in the maintainer's clone), as
/// `languages.md` tabulates them. A finding counts for the language its path
/// names, as `analysis::generic::preview` names it; a rule and level without
/// a row had no finding there. The ten supported languages' shares in
/// [`TABLE`] say little of these: Bash's shared-logic reviews were right 4
/// times in 34 and its function-simplification reviews 25 in 28, where the
/// ten's were right 46 in 85 and 20 in 23.
const PREVIEW: [PreviewMeasure; 53] = [
    preview_row("C", FUNCTION_SIMPLIFICATION, Review, [10, 11]),
    preview_row("C", FUNCTION_SIMPLIFICATION, Consider, [13, 19]),
    preview_row("C", SHARED_LOGIC, Review, [6, 14]),
    preview_row("C", SHARED_LOGIC, Consider, [4, 10]),
    preview_row("C", COMMENTS, Consider, [0, 8]),
    preview_row("C", FILE_ORGANIZATION, Consider, [0, 1]),
    preview_row("C++", FUNCTION_SIMPLIFICATION, Review, [11, 11]),
    preview_row("C++", FUNCTION_SIMPLIFICATION, Consider, [19, 37]),
    preview_row("C++", SHARED_LOGIC, Review, [12, 29]),
    preview_row("C++", SHARED_LOGIC, Consider, [3, 18]),
    preview_row("C++", COMMENTS, Consider, [1, 6]),
    preview_row("C++", FILE_ORGANIZATION, Consider, [1, 3]),
    preview_row("Kotlin", FUNCTION_SIMPLIFICATION, Review, [1, 1]),
    preview_row("Kotlin", FUNCTION_SIMPLIFICATION, Consider, [6, 7]),
    preview_row("Kotlin", SHARED_LOGIC, Review, [6, 7]),
    preview_row("Kotlin", SHARED_LOGIC, Consider, [2, 3]),
    preview_row("Kotlin", COMMENTS, Consider, [1, 2]),
    preview_row("Kotlin", FILE_ORGANIZATION, Review, [1, 1]),
    preview_row("Swift", FUNCTION_SIMPLIFICATION, Review, [10, 10]),
    preview_row("Swift", FUNCTION_SIMPLIFICATION, Consider, [22, 30]),
    preview_row("Swift", SHARED_LOGIC, Review, [17, 23]),
    preview_row("Swift", SHARED_LOGIC, Consider, [18, 40]),
    preview_row("Swift", COMMENTS, Consider, [4, 4]),
    preview_row("Swift", FILE_ORGANIZATION, Review, [1, 1]),
    preview_row("Swift", FILE_ORGANIZATION, Consider, [2, 2]),
    preview_row("Bash", FUNCTION_SIMPLIFICATION, Review, [25, 28]),
    preview_row("Bash", FUNCTION_SIMPLIFICATION, Consider, [34, 50]),
    preview_row("Bash", SHARED_LOGIC, Review, [4, 34]),
    preview_row("Bash", SHARED_LOGIC, Consider, [0, 11]),
    preview_row("Bash", COMMENTS, Consider, [22, 28]),
    preview_row("Bash", FILE_ORGANIZATION, Review, [0, 2]),
    preview_row("Bash", FILE_ORGANIZATION, Consider, [0, 2]),
    preview_row("Dart", FUNCTION_SIMPLIFICATION, Review, [5, 5]),
    preview_row("Dart", FUNCTION_SIMPLIFICATION, Consider, [9, 10]),
    preview_row("Dart", SHARED_LOGIC, Review, [3, 7]),
    preview_row("Dart", SHARED_LOGIC, Consider, [3, 4]),
    preview_row("Dart", COMMENTS, Consider, [2, 4]),
    preview_row("Dart", FILE_ORGANIZATION, Consider, [1, 1]),
    preview_row("Scala", FUNCTION_SIMPLIFICATION, Review, [2, 3]),
    preview_row("Scala", FUNCTION_SIMPLIFICATION, Consider, [5, 11]),
    preview_row("Scala", SHARED_LOGIC, Review, [1, 2]),
    preview_row("Scala", SHARED_LOGIC, Consider, [1, 3]),
    preview_row("Scala", COMMENTS, Consider, [3, 5]),
    preview_row("Scala", FILE_ORGANIZATION, Consider, [0, 3]),
    preview_row("Elixir", FUNCTION_SIMPLIFICATION, Consider, [7, 8]),
    preview_row("Elixir", SHARED_LOGIC, Review, [4, 5]),
    preview_row("Elixir", SHARED_LOGIC, Consider, [3, 7]),
    preview_row("Elixir", FILE_ORGANIZATION, Review, [1, 1]),
    preview_row("Lua", FUNCTION_SIMPLIFICATION, Review, [11, 12]),
    preview_row("Lua", FUNCTION_SIMPLIFICATION, Consider, [18, 22]),
    preview_row("Lua", SHARED_LOGIC, Review, [8, 9]),
    preview_row("Lua", SHARED_LOGIC, Consider, [1, 8]),
    preview_row("Lua", COMMENTS, Consider, [1, 10]),
];

/// The preview language whose own labels weigh a finding of `rule` in the
/// file at `path`, and whose findings never fail the default gate: the
/// file's language when it is in preview (`analysis::generic`). None for a
/// supported language, and for a custom question, which its team measures
/// by its examples and which fails at its own level in every language.
pub fn preview_language(path: &Path, rule: &str) -> Option<&'static str> {
    crate::analysis::generic::preview(path).filter(|_| !catalog::custom(rule))
}

/// The labels a finding of `rule` at `level` in the file at `path`
/// carries: in a preview language that language's own ([`PREVIEW`]), none
/// labeled where it has none; elsewhere [`precision`].
pub fn precision_at(path: &Path, rule: &str, level: Strength) -> Option<Labels> {
    let Some(language) = preview_language(path, rule) else {
        return precision(rule, level);
    };
    let key = catalog::find(rule).map(|found| found.key);
    (level != Strength::Note).then(|| {
        PREVIEW
            .iter()
            .find(|m| m.language == language && Some(m.rule) == key && m.level == level)
            .map_or_else(Labels::default, |m| m.unseen)
    })
}

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

/// Where the labels behind `rule`'s levels come from.
pub fn dataset(rule: &str) -> String {
    if bend_2_only(rule) {
        format!("findings {BEND_2_ONLY}")
    } else {
        format!(
            "findings labeled from the code on the corpus: 25 projects JevGate was never tuned on (`unseen`) and the projects it was tuned on (`tuned`); {}accuracy.html",
            catalog::SITE
        )
    }
}

/// How often findings of `rule` with `labels` were right, for a reader:
/// [`Labels::in_words`], in a preview `language` naming it, since the labels
/// are its own; and for a law finding, which has none, that its labels are
/// Bend 2's: "not yet measured" alone would say none were made.
pub fn precision_in_words(rule: &str, labels: Labels, language: Option<&str>) -> String {
    if let Some(language) = language {
        return labels.in_words_in(language);
    }
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
        let unseen = |rule| measure(rule, Review).unwrap().unseen;
        assert_eq!(
            unseen(catalog::SHARED_LOGIC).summary().as_deref(),
            Some("54% of 85")
        );
        let few = unseen(catalog::HARDCODED_VALUES);
        assert_eq!(
            few.summary().as_deref(),
            Some("1 of 8"),
            "no share below 20"
        );
        assert_eq!(few.percent(), Some(13), "12.5% rounds up");
        assert_eq!(unseen(catalog::ACCESS_CONTROL).summary(), None);
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
    fn a_preview_language_s_finding_carries_that_language_s_own_labels() {
        let at = |path: &str, rule, level| precision_at(Path::new(path), rule, level);
        let labels = |right, labeled| Some(Labels { right, labeled });
        assert_eq!(
            at("View.swift", catalog::FUNCTION_SIMPLIFICATION, Consider),
            labels(22, 30)
        );
        assert_eq!(
            at("Shop.kt", catalog::COMMENTS, Review),
            labels(0, 0),
            "no such finding there"
        );
        assert_eq!(
            at("src/lib.rs", catalog::FUNCTION_SIMPLIFICATION, Review),
            labels(20, 23),
            "a supported language's are the table's"
        );
        assert_eq!(at("Shop.kt", catalog::SHARED_LOGIC, Strength::Note), None);
        assert_eq!(
            preview_language(Path::new("Shop.kt"), "custom/body-logs"),
            None,
            "a team's question is measured by its examples"
        );
        assert_eq!(
            precision_in_words(
                catalog::SHARED_LOGIC,
                Labels {
                    right: 22,
                    labeled: 30
                },
                Some("Swift")
            ),
            "right 73% of the time in Swift (30 labels)"
        );
        assert_eq!(
            Labels {
                right: 1,
                labeled: 1
            }
            .in_words_in("Kotlin"),
            "not yet measured in Kotlin"
        );
    }

    #[test]
    fn the_preview_table_sums_to_each_language_s_published_counts() {
        // `languages.md`'s support levels: reviews, then considers, right of
        // labeled; and a file of each language's.
        let published = [
            ("C", "x.c", [16, 25], [17, 38]),
            ("C++", "x.cpp", [23, 40], [24, 64]),
            ("Kotlin", "x.kt", [8, 9], [9, 12]),
            ("Swift", "x.swift", [28, 34], [46, 76]),
            ("Bash", "x.sh", [29, 64], [56, 91]),
            ("Dart", "x.dart", [8, 12], [15, 19]),
            ("Scala", "x.scala", [3, 5], [9, 22]),
            ("Elixir", "x.ex", [5, 6], [10, 15]),
            ("Lua", "x.lua", [19, 21], [20, 40]),
        ];
        let keys = catalog::keys();
        for m in &PREVIEW {
            assert!(keys.contains(&m.rule), "{}", m.rule);
            assert!(
                published
                    .iter()
                    .any(|(language, ..)| *language == m.language)
            );
        }
        for (language, file, reviews, considers) in published {
            assert_eq!(
                crate::analysis::generic::preview(Path::new(file)),
                Some(language)
            );
            let sum = |level: Strength| {
                PREVIEW
                    .iter()
                    .filter(|m| m.language == language && m.level == level)
                    .fold([0, 0], |[right, labeled], m| {
                        [right + m.unseen.right, labeled + m.unseen.labeled]
                    })
            };
            assert_eq!(
                (sum(Review), sum(Consider)),
                (reviews, considers),
                "{language}"
            );
        }
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
            precision_in_words(catalog::LAWS, never, None),
            "not yet measured: labeled only on Bend 2 projects, which the maturity table leaves out"
        );
        let none = labels(catalog::ACCESS_CONTROL, Consider);
        assert_eq!(
            precision_in_words(catalog::ACCESS_CONTROL, none, None),
            "not yet measured"
        );
        assert_eq!(precision(catalog::SHARED_LOGIC, Strength::Note), None);
    }
}
