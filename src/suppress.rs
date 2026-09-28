//! Inline suppressions: a comment `jevgate: allow(RULE, …) reason` on a
//! finding's line, or in the comments and attributes directly above it,
//! accepts that finding as the baseline does. RULE is a rule ID, name, key or group,
//! and the reason is required: without one the comment is ignored and the
//! finding says so.
use crate::{catalog, schema::Report};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

const MARKER: &str = "jevgate:";

/// What one `jevgate: allow(…)` comment names and why.
#[derive(Debug, PartialEq)]
struct Allow {
    rules: Vec<String>,
    reason: String,
}

/// Mark the findings an allow comment names, but for comments on the
/// `ignored` lines (a file and a 1-based line), which accept nothing. Files
/// that cannot be read keep their findings.
pub fn apply(root: &Path, report: &mut Report, ignored: &BTreeSet<(PathBuf, usize)>) {
    for file in report.files.iter_mut().filter(|f| !f.findings.is_empty()) {
        let Ok(text) = std::fs::read_to_string(root.join(&file.path)) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        let skipped: BTreeSet<usize> = ignored
            .iter()
            .filter(|(path, _)| *path == file.path)
            .map(|(_, line)| *line)
            .collect();
        let skipped = |line: usize| skipped.contains(&line);
        for finding in &mut file.findings {
            finding.suppressed = None;
            let Some(allow) = allow_for(&lines, finding.line, &finding.rule, skipped) else {
                continue;
            };
            if allow.reason.is_empty() {
                finding.message.push_str(
                    " The `jevgate: allow` comment for it is ignored: it gives no reason after the rule.",
                );
            } else {
                finding.suppressed = Some(allow.reason);
            }
        }
    }
}

/// Whether `line` holds an allow comment that accepts findings, as `apply`
/// reads it: one naming a rule and giving a reason.
pub fn accepts(line: &str) -> bool {
    parse(line).is_some_and(|allow| {
        !allow.reason.is_empty()
            && allow
                .rules
                .iter()
                .any(|name| catalog::select(name).is_some())
    })
}

/// The allow comment naming `rule` on 1-based `line`, or in the block of
/// comment and attribute lines directly above it, but on no `skipped` line.
fn allow_for(
    lines: &[&str],
    line: usize,
    rule: &str,
    skipped: impl Fn(usize) -> bool,
) -> Option<Allow> {
    let at = line.checked_sub(1)?;
    let above = lines[..at.min(lines.len())]
        .iter()
        .enumerate()
        .rev()
        .take_while(|(_, l)| annotation(l));
    lines
        .get(at)
        .map(|l| (at, l))
        .into_iter()
        .chain(above)
        .filter(|(index, _)| !skipped(index + 1))
        .filter_map(|(_, l)| parse(l))
        .find(|allow| allow.rules.iter().any(|name| names(name, rule)))
}

/// A comment, attribute or decorator line, which may sit between an allow
/// comment and the code it is about.
pub(crate) fn annotation(line: &str) -> bool {
    let line = line.trim_start();
    ["//", "#", "/*", "*", "--", "<!--", "@", "["]
        .iter()
        .any(|start| line.starts_with(start))
}

/// Whether `name` (an ID, name, key or group) selects the rule with ID
/// `rule`. A custom question is named by its ID, its group, `default` or
/// `all`, never by the id alone.
fn names(name: &str, rule: &str) -> bool {
    if catalog::custom(rule) {
        return name == rule
            || [
                catalog::CUSTOM_GROUP,
                catalog::DEFAULT_GROUP,
                catalog::ALL_GROUP,
            ]
            .contains(&name);
    }
    let key = catalog::find(rule).map(|r| r.key);
    catalog::select(name).is_some_and(|keys| key.is_some_and(|key| keys.contains(&key)))
}

fn parse(line: &str) -> Option<Allow> {
    let rest = line[line.find(MARKER)? + MARKER.len()..].trim_start();
    let rest = rest.strip_prefix("allow")?.trim_start().strip_prefix('(')?;
    let (inside, after) = rest.split_once(')')?;
    let rules = inside
        .split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .collect();
    let reason = after
        .trim()
        .trim_end_matches("*/")
        .trim_end_matches("-->")
        .trim_end_matches("%>")
        .trim()
        .trim_start_matches(['-', ':', '—'])
        .trim();
    Some(Allow {
        rules,
        reason: reason.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_comments_name_rules_and_a_reason_in_any_comment_style() {
        for (line, rules, reason) in [
            (
                "// jevgate: allow(shared_logic) the two exports must stay separate",
                vec!["shared_logic"],
                "the two exports must stay separate",
            ),
            (
                "    # jevgate: allow(maintainability/hardcoded-values, security) -- test fixture",
                vec!["maintainability/hardcoded-values", "security"],
                "test fixture",
            ),
            (
                "/* jevgate: allow(injection): the query is a constant */",
                vec!["injection"],
                "the query is a constant",
            ),
            ("<!-- jevgate:allow(comments) -->", vec!["comments"], ""),
        ] {
            assert_eq!(
                parse(line),
                Some(Allow {
                    rules: rules.into_iter().map(String::from).collect(),
                    reason: reason.into()
                }),
                "{line}"
            );
        }
        assert_eq!(parse("// jevgate: allow shared_logic"), None);
        assert_eq!(parse("let jevgate = 1;"), None);
        assert!(accepts(
            "# jevgate: allow(injection) the query is a constant"
        ));
        assert!(!accepts("// jevgate: allow(injection)"), "no reason");
        assert!(!accepts(
            "/// a line that holds a `jevgate: allow(…)` comment"
        ));
    }

    #[test]
    fn an_allow_comment_applies_on_its_line_or_through_the_annotations_above() {
        let source = [
            "use std::fs;",
            "// jevgate: allow(maintainability) generated shape we keep",
            "/// Loads the file.",
            "#[inline]",
            "fn load() {}",
            "",
            "fn other() {} // jevgate: allow(shared_logic) mirrors load",
        ];
        let rule = "maintainability/shared-logic";
        let none = |_| false;
        assert!(
            allow_for(&source, 5, rule, none).is_some(),
            "through a doc comment and attribute"
        );
        assert!(
            allow_for(&source, 7, rule, none).is_some(),
            "at the end of the line"
        );
        assert!(allow_for(&source, 7, "security/injection", none).is_none());
        assert!(allow_for(&source, 1, rule, none).is_none());
        assert!(
            allow_for(&source, 5, rule, |line| line == 2).is_none(),
            "a skipped comment accepts nothing"
        );
        let apart = ["// jevgate: allow(shared_logic) old", "", "fn load() {}"];
        assert!(
            allow_for(&apart, 3, rule, none).is_none(),
            "a blank line ends the block"
        );
    }
}
