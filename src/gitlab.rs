//! `--format gitlab`: the findings as a GitLab Code Quality report, which
//! merge requests show as a widget and on the changed lines.
use crate::{
    options::CheckArgs,
    output,
    schema::{Finding, Report, Strength},
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{io::Write, path::Path};

/// The findings the GitHub annotations show: `major` when a finding fails the
/// gate, `minor` otherwise.
pub fn emit(out: &mut impl Write, report: &Report, args: &CheckArgs) -> Result<()> {
    let issues: Vec<Value> = output::ranked(report)
        .into_iter()
        .filter(|(_, f)| f.strength != Strength::Note && !f.baselined)
        .map(|(path, finding)| issue(path, finding, crate::gate::fails(finding, path, args)))
        .collect();
    serde_json::to_writer_pretty(&mut *out, &issues)?;
    writeln!(out)?;
    Ok(())
}

fn issue(path: &Path, finding: &Finding, fails: bool) -> Value {
    let end = finding
        .locations
        .iter()
        .find(|l| l.path == path && l.start_line == finding.line)
        .map_or(finding.line, |l| l.end_line.max(finding.line));
    json!({
        "description": format!("{} Next step: {}", finding.message, finding.action),
        "check_name": finding.rule,
        "fingerprint": fingerprint(path, finding),
        "severity": if fails { "major" } else { "minor" },
        "location": {
            "path": path.to_string_lossy(),
            "lines": {"begin": finding.line.max(1), "end": end.max(1)},
        },
    })
}

/// GitLab requires a fingerprint per issue; findings carry JevGate's, and one
/// without is named by its rule and place.
fn fingerprint(path: &Path, finding: &Finding) -> String {
    if !finding.fingerprint.is_empty() {
        return finding.fingerprint.clone();
    }
    let place = [
        finding.rule.clone(),
        path.display().to_string(),
        finding.line.to_string(),
    ]
    .join(crate::schema::HASH_SEPARATOR);
    crate::schema::hash(place.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::finding;

    #[test]
    fn issues_carry_severity_location_and_a_fingerprint() {
        let path = Path::new("src/a,b.rs");
        let review = issue(path, &finding(Strength::Review), true);
        assert_eq!(review["severity"], "major");
        assert_eq!(review["check_name"], "maintainability/shared-logic");
        assert_eq!(review["location"]["path"], "src/a,b.rs");
        assert_eq!(review["location"]["lines"], json!({"begin": 12, "end": 20}));
        assert!(
            review["description"]
                .as_str()
                .unwrap()
                .ends_with("Next step: Share one | implementation")
        );
        assert_eq!(review["fingerprint"].as_str().unwrap().len(), 64);
        let consider = issue(path, &finding(Strength::Consider), false);
        assert_eq!(consider["severity"], "minor");
        let mut named = finding(Strength::Consider);
        named.fingerprint = "abc".into();
        assert_eq!(issue(path, &named, false)["fingerprint"], "abc");
    }
}
