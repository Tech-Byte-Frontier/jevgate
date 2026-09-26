//! Inline suppressions: a comment `jevgate: allow(RULE, …) reason` on a
//! finding's line, or in the comments and attributes directly above it,
//! accepts that finding as the baseline does. RULE is a rule ID, name, key or group,
//! and the reason is required: without one the comment is ignored and the
//! finding says so.
use crate::{catalog, schema::Report};
use std::path::Path;

const MARKER: &str = "jevgate:";

/// What one `jevgate: allow(…)` comment names and why.
#[derive(Debug, PartialEq)]
struct Allow {
    rules: Vec<String>,
    reason: String,
}

/// Mark the findings an allow comment names. Files that cannot be read keep
/// their findings.
pub fn apply(root: &Path, report: &mut Report) {
    for file in report.files.iter_mut().filter(|f| !f.findings.is_empty()) {
        let Ok(text) = std::fs::read_to_string(root.join(&file.path)) else {
            continue;
        };
        let lines: Vec<&str> = text.lines().collect();
        for finding in &mut file.findings {
            finding.suppressed = None;
            let Some(allow) = allow_for(&lines, finding.line, &finding.rule) else {
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

/// The allow comment naming `rule` on 1-based `line`, or in the block of
/// comment and attribute lines directly above it.
fn allow_for(lines: &[&str], line: usize, rule: &str) -> Option<Allow> {
    let at = line.checked_sub(1)?;
    let above = lines[..at.min(lines.len())]
        .iter()
        .rev()
        .take_while(|l| annotation(l));
    lines
        .get(at)
        .into_iter()
        .chain(above)
        .filter_map(|l| parse(l))
        .find(|allow| allow.rules.iter().any(|name| names(name, rule)))
}

/// A comment, attribute or decorator line, which may sit between an allow
/// comment and the code it is about.
fn annotation(line: &str) -> bool {
    let line = line.trim_start();
    ["//", "#", "/*", "*", "--", "<!--", "@", "["]
        .iter()
        .any(|start| line.starts_with(start))
}

/// Whether `name` (an ID, name, key or group) selects the rule with ID `rule`.
fn names(name: &str, rule: &str) -> bool {
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
        assert!(
            allow_for(&source, 5, rule).is_some(),
            "through a doc comment and attribute"
        );
        assert!(
            allow_for(&source, 7, rule).is_some(),
            "at the end of the line"
        );
        assert!(allow_for(&source, 7, "security/injection").is_none());
        assert!(allow_for(&source, 1, rule).is_none());
        let apart = ["// jevgate: allow(shared_logic) old", "", "fn load() {}"];
        assert!(
            allow_for(&apart, 3, rule).is_none(),
            "a blank line ends the block"
        );
    }
}
