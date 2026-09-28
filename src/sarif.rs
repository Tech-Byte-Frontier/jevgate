//! `--format sarif`: the findings as a SARIF 2.1.0 log, for GitHub code
//! scanning, GitLab and editors that read static analysis results.
use crate::{
    catalog, output,
    schema::{Finding, Report, Status, Strength},
};
use anyhow::Result;
use serde_json::{Value, json};
use std::{io::Write, path::Path};

const SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";
const HOME: &str = "https://github.com/Tech-Byte-Frontier/jevgate";

/// The same findings the GitHub annotations show: every new finding that is
/// not a note, an `error` when it fails the gate and a `warning` otherwise,
/// with how the gate counted it as the `gate` property and how often its
/// rule and level were right as `precision`. Run errors and files that could
/// not be judged are tool notifications.
pub fn emit(out: &mut impl Write, report: &Report) -> Result<()> {
    let shown: Vec<(&Path, &Finding)> = output::ranked(report)
        .into_iter()
        .filter(|(_, f)| f.strength != Strength::Note && !f.accepted())
        .collect();
    serde_json::to_writer_pretty(&mut *out, &document(report, &shown))?;
    writeln!(out)?;
    Ok(())
}

fn document(report: &Report, shown: &[(&Path, &Finding)]) -> Value {
    let rules = catalog::rules();
    let results: Vec<Value> = shown
        .iter()
        .map(|(path, finding)| {
            let index = rules.iter().position(|r| r.id == finding.rule);
            result(path, finding, index)
        })
        .collect();
    let mut notifications: Vec<Value> = report
        .errors
        .iter()
        .map(|error| json!({"level": "error", "message": {"text": error}}))
        .collect();
    notifications.extend(
        report
            .files
            .iter()
            .filter(|f| f.status == Status::Error)
            .map(|file| {
                json!({
                    "level": "error",
                    "message": {"text": file.error.as_deref().unwrap_or("Not judged")},
                    "locations": [{"physicalLocation": {"artifactLocation": artifact(&file.path)}}],
                })
            }),
    );
    json!({
        "$schema": SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": "JevGate",
                "version": env!("CARGO_PKG_VERSION"),
                "semanticVersion": env!("CARGO_PKG_VERSION"),
                "informationUri": HOME,
                "rules": rules.iter().map(rule).collect::<Vec<_>>(),
            }},
            "invocations": [{
                "executionSuccessful": report.complete,
                "toolExecutionNotifications": notifications,
            }],
            "results": results,
        }],
    })
}

/// A rule's reporting descriptor: its question as the description, and its
/// group as a tag; code scanning lists rules tagged `security` as security alerts.
fn rule(rule: &catalog::Rule) -> Value {
    json!({
        "id": rule.id,
        "name": rule.key,
        "shortDescription": {"text": title(rule.id)},
        "fullDescription": {"text": rule.inspection},
        "help": {"text": format!("{}\n\nAcceptable: {}", rule.inspection, rule.acceptable_example)},
        "helpUri": format!("{HOME}#what-it-finds"),
        "properties": {"tags": [rule.group]},
    })
}

/// `maintainability/shared-logic` → `Shared logic`.
fn title(id: &str) -> String {
    let name = id.rsplit('/').next().unwrap_or(id).replace('-', " ");
    let mut chars = name.chars();
    chars.next().map_or(String::new(), |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn result(path: &Path, finding: &Finding, rule_index: Option<usize>) -> Value {
    let end = finding
        .locations
        .iter()
        .find(|l| l.path == path && l.start_line == finding.line)
        .map_or(finding.line, |l| l.end_line.max(finding.line));
    let related: Vec<Value> = finding
        .locations
        .iter()
        .filter(|l| !(l.path == path && l.start_line == finding.line))
        .enumerate()
        .map(|(id, l)| {
            json!({
                "id": id,
                "physicalLocation": {
                    "artifactLocation": artifact(&l.path),
                    "region": {"startLine": l.start_line.max(1), "endLine": l.end_line.max(l.start_line).max(1)},
                },
            })
        })
        .collect();
    let mut text = format!(
        "{}\n\nNext step: {}",
        output::claim(finding, output::Style::PLAIN),
        finding.action
    );
    if let Some(note) = output::measuring_note(finding) {
        text.push_str(&format!("\n\n{note}"));
    }
    let mut value = json!({
        "ruleId": finding.rule,
        "level": if finding.fails_gate() { "error" } else { "warning" },
        "message": {"text": text},
        "locations": [{"physicalLocation": {
            "artifactLocation": artifact(path),
            "region": {"startLine": finding.line.max(1), "endLine": end.max(1)},
        }}],
        "partialFingerprints": {"jevgateFingerprint/v1": finding.fingerprint},
        "properties": {
            "strength": output::label(&finding.strength),
            "probability": finding.concern_probability,
        },
    });
    if let Some(index) = rule_index {
        value["ruleIndex"] = json!(index);
    }
    if !related.is_empty() {
        value["relatedLocations"] = json!(related);
    }
    if let Some(category) = &finding.category {
        value["properties"]["category"] = json!(category);
    }
    if let Some(gate) = finding.gate {
        value["properties"]["gate"] = json!(gate);
    }
    if let Some(precision) = finding.precision {
        value["properties"]["precision"] = json!(precision);
    }
    value
}

/// A repository-relative file, as code scanning matches it to the checkout.
fn artifact(path: &Path) -> Value {
    json!({"uri": path.to_string_lossy(), "uriBaseId": "%SRCROOT%"})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{options::CheckArgs, schema::Gating, tests::counted};

    fn report(args: &CheckArgs) -> Report {
        crate::evaluate::snapshot(
            &[],
            &Default::default(),
            args,
            crate::evaluate::SnapshotContext {
                root: Path::new("."),
                generation: 1,
                requests: 0,
            },
        )
    }

    #[test]
    fn results_name_their_rule_level_location_and_fingerprint() {
        let args = crate::tests::args();
        let review = counted(Strength::Review, Gating::Fails);
        let measuring = counted(Strength::Review, Gating::Measuring);
        let path = Path::new("src/a,b.rs");
        let log = document(&report(&args), &[(path, &review), (path, &measuring)]);
        assert_eq!(log["version"], "2.1.0");
        let run = &log["runs"][0];
        let rules = run["tool"]["driver"]["rules"].as_array().unwrap();
        assert_eq!(rules.len(), catalog::rules().len());
        let results = run["results"].as_array().unwrap();
        assert_eq!(results[0]["level"], "error", "a review that fails the gate");
        assert_eq!(results[0]["properties"]["gate"], "fails");
        assert_eq!(results[1]["level"], "warning");
        assert_eq!(results[1]["properties"]["gate"], "measuring");
        assert!(
            results[1]["message"]["text"]
                .as_str()
                .unwrap()
                .ends_with("Does not fail the gate: by default only rules and levels right at least 80% of the time over at least 20 labels on projects JevGate was never tuned on fail it.")
        );
        assert_eq!(
            results[0]["properties"]["precision"],
            json!({"right": 46, "labeled": 85})
        );
        assert_eq!(results[0]["properties"]["probability"], 0.9);
        let first = &results[0];
        let index = first["ruleIndex"].as_u64().unwrap() as usize;
        assert_eq!(rules[index]["id"], "maintainability/shared-logic");
        assert_eq!(rules[index]["shortDescription"]["text"], "Shared logic");
        let region = &first["locations"][0]["physicalLocation"];
        assert_eq!(region["artifactLocation"]["uri"], "src/a,b.rs");
        assert_eq!(region["region"]["startLine"], 12);
        assert_eq!(region["region"]["endLine"], 20);
        assert!(first.get("relatedLocations").is_none());
        assert!(first["message"]["text"].as_str().unwrap().ends_with(
            "Right 54% of the time (85 labels).\n\nNext step: Share one | implementation"
        ));
        assert!(first["partialFingerprints"]["jevgateFingerprint/v1"].is_string());
    }

    #[test]
    fn security_rules_are_tagged_for_code_scanning() {
        let args = crate::tests::args();
        let log = document(&report(&args), &[]);
        let rules = log["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap();
        let injection = rules
            .iter()
            .find(|r| r["id"] == "security/injection")
            .unwrap();
        assert_eq!(injection["properties"]["tags"], json!(["security"]));
        assert_eq!(log["runs"][0]["results"], json!([]));
    }
}
