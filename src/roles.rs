//! Semantic roles are independent observations, never maintainability verdicts.
use crate::{
    inventory::Input,
    options::CheckArgs,
    schema::{FileResult, Status},
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const VERSION: &str = "region-roles-v6";
const LIMIT: usize = 32;
const PRESENT: f64 = 0.8;
const ABSENT: f64 = 0.2;
const EVIDENCE: f64 = 0.5;

pub fn policy() -> BTreeMap<&'static str, f64> {
    BTreeMap::from([
        ("role_present", PRESENT),
        ("role_absent", ABSENT),
        ("role_evidence_missing_below", EVIDENCE),
        ("role_evidence_resolved", PRESENT),
    ])
}
pub const ROLES: [&str; 4] = [
    "test_scenario",
    "test_support",
    "framework_tool",
    "application_library",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assessment {
    pub version: String,
    pub model: String,
    pub regions: Vec<Value>,
    pub relationships: Vec<Value>,
    pub limitations: Value,
}

pub fn request(input: &Input, args: &CheckArgs) -> Result<Value> {
    build_request(input, args, true)
}

pub fn occurrence_request(input: &Input, args: &CheckArgs) -> Result<Value> {
    build_request(input, args, false)
}

fn build_request(input: &Input, args: &CheckArgs, include_other_regions: bool) -> Result<Value> {
    let source = input.source.as_deref().context("Missing selected source")?;
    let sources: Vec<_> = std::iter::once((input.result.path.as_path(), source))
        .chain(
            input
                .context
                .iter()
                .map(|c| (c.file.path.as_path(), c.source.as_str())),
        )
        .collect();
    let mut units = Vec::new();
    let mut unsupported = Vec::new();
    for (path, text) in &sources {
        if crate::locations::parse(path, text)?.is_none() {
            unsupported.push(json!(path));
        }
        let targets = crate::context_units::review_targets(path, text);
        if targets.is_empty() {
            units.push(json!({"path":path,"start_line":1,"end_line":text.lines().count().max(1),"name":"file region","kind":"file"}));
        } else {
            for (name, range, _) in targets {
                units.push(json!({"path":path,"start_line":range.start_line,"end_line":range.end_line,"name":name,"kind":"operation"}));
            }
        }
    }
    let repetition = crate::repetition::observations(sources.iter().copied())?;
    let mut regions = Vec::<Value>::new();
    let mut omitted = BTreeSet::new();
    let mut add = |region: Value| -> Option<usize> {
        if let Some(index) = regions.iter().position(|r| r == &region) {
            return Some(index);
        }
        if regions.len() == LIMIT {
            omitted.insert(region.to_string());
            return None;
        }
        let index = regions.len();
        regions.push(region);
        Some(index)
    };
    let mut fragments = Vec::new();
    for fragment in repetition["observations"].as_array().unwrap() {
        let mut occurrences = Vec::new();
        for location in fragment["locations"].as_array().unwrap() {
            let enclosing = units
                .iter()
                .filter(|u| {
                    u["path"] == location["path"]
                        && u["start_line"].as_u64() <= location["start_line"].as_u64()
                        && u["end_line"].as_u64() >= location["end_line"].as_u64()
                })
                .min_by_key(|u| {
                    u["end_line"].as_u64().unwrap() - u["start_line"].as_u64().unwrap()
                });
            let region = enclosing.cloned().unwrap_or_else(|| json!({"path":location["path"],"start_line":location["start_line"],"end_line":location["end_line"],"name":"cross-boundary occurrence","kind":"occurrence"}));
            occurrences.push(json!({"location":location,"region_index":add(region)}));
        }
        fragments.push(json!({"occurrences":occurrences,"locations_omitted":fragment["locations_omitted"].as_u64().unwrap_or(0)}));
    }
    if include_other_regions {
        for unit in units {
            add(unit);
        }
    }
    let mut excerpt_budget = 16384usize;
    let mut excerpts_omitted = 0;
    let region_sources: Vec<Value> = regions
        .iter()
        .map(|region| {
            let text = sources
                .iter()
                .find(|(path, _)| json!(path) == region["path"])
                .unwrap()
                .1;
            let start = region["start_line"].as_u64().unwrap() as usize;
            let end = region["end_line"].as_u64().unwrap() as usize;
            let excerpt = text
                .lines()
                .skip(start - 1)
                .take(end - start + 1)
                .collect::<Vec<_>>()
                .join("\n");
            if excerpt.len() <= excerpt_budget {
                excerpt_budget -= excerpt.len();
                json!(excerpt)
            } else {
                excerpts_omitted += 1;
                Value::Null
            }
        })
        .collect();
    let mut questions = serde_json::Map::new();
    for index in 0..regions.len() {
        for role in ROLES {
            questions.insert(
                format!("role_{index}_{role}"),
                crate::questions::role_noul(index, role),
            );
        }
        questions.insert(
            format!("role_{index}_evidence"),
            crate::questions::evidence_noul(index),
        );
    }
    Ok(json!({"model":args.model,"state":{"role_version":VERSION,
        "file":{"path":input.result.path,"source":source,"source_hash":input.result.source_hash},
        "context":input.context.iter().map(|c| json!({"path":c.file.path,"source":c.source,"source_hash":c.file.source_hash})).collect::<Vec<_>>(),
        "regions":regions,"region_sources":region_sources,"fragments":fragments,
        "limitations":{"region_excerpts_omitted":excerpts_omitted,"regions_omitted":omitted.len(),"unsupported_parser_paths":unsupported,
            "repetition_scope":repetition["scope"],"repetition_candidate_count":repetition["candidate_count"],
            "scope":"Selected source and explicit context only. At most 32 deduplicated regions, prioritizing fragment occurrences. Nested callbacks can share their containing operation; roles may overlap. Missing candidates are not negative classifications."}},"questions":questions}))
}

fn label(probability: f64, sufficient: f64) -> &'static str {
    if sufficient < EVIDENCE {
        "needs-context"
    } else if !crate::response::probability_at_least(sufficient, PRESENT) {
        "uncertain"
    } else if crate::response::probability_at_least(probability, PRESENT) {
        "present"
    } else if crate::response::probability_at_least(1.0 - probability, 1.0 - ABSENT) {
        "absent"
    } else {
        "uncertain"
    }
}

fn families(region: &Value) -> BTreeSet<&'static str> {
    let mut result = BTreeSet::new();
    for (role, family) in [
        ("test_scenario", "test"),
        ("test_support", "test"),
        ("framework_tool", "implementation"),
        ("application_library", "implementation"),
    ] {
        if region["roles"][role]["status"] == "present" {
            result.insert(family);
        }
    }
    result
}

fn relationships(fragments: &Value, regions: &[Value]) -> Vec<Value> {
    fragments.as_array().unwrap().iter().enumerate().map(|(index, fragment)| {
        let occurrences = fragment["occurrences"].as_array().unwrap();
        let mut pairs = Vec::new();
        for a in 0..occurrences.len() {
            for b in a+1..occurrences.len() {
                let left = occurrences[a]["region_index"].as_u64().and_then(|i| regions.get(i as usize));
                let right = occurrences[b]["region_index"].as_u64().and_then(|i| regions.get(i as usize));
                let mut kinds = BTreeSet::new();
                if let (Some(left), Some(right)) = (left, right) {
                    for l in families(left) { for r in families(right) {
                        kinds.insert(match (l,r) { ("test","test") => "test-test", ("implementation","implementation") => "implementation-implementation", _ => "test-implementation" });
                    }}
                }
                let unresolved = [left,right].iter().any(|r| r.is_none_or(|r| r["status"] != "classified")) || kinds.is_empty();
                pairs.push(json!({"left_occurrence":a,"right_occurrence":b,"kinds":kinds,"unresolved":unresolved,"fallback":"general assessment remains available; roles do not establish maintainability"}));
            }
        }
        json!({"fragment_index":index,"occurrences":occurrences,"pairs":pairs,"locations_omitted":fragment["locations_omitted"]})
    }).collect()
}

pub fn assess(state: &Value, body: &Value) -> Result<Assessment> {
    let mut regions = Vec::new();
    for (index, region) in state["regions"]
        .as_array()
        .context("Missing regions")?
        .iter()
        .enumerate()
    {
        let evidence = &body["answers"][format!("role_{index}_evidence")];
        let sufficient = evidence["noul"]
            .as_f64()
            .context("Missing evidence probability")?;
        let mut roles = BTreeMap::new();
        for role in ROLES {
            let answer = &body["answers"][format!("role_{index}_{role}")];
            let probability = answer["noul"]
                .as_f64()
                .context("Missing role probability")?;
            roles.insert(
                role,
                json!({"answer":answer,"status":label(probability,sufficient)}),
            );
        }
        let status = if sufficient < EVIDENCE {
            "needs-context"
        } else if roles.values().any(|r| r["status"] == "uncertain") {
            "uncertain"
        } else {
            "classified"
        };
        regions.push(json!({"index":index,"evidence":region,"evidence_sufficiency":evidence,"roles":roles,"status":status}));
    }
    Ok(Assessment {
        version: VERSION.into(),
        model: body["model"].as_str().unwrap_or_default().into(),
        relationships: relationships(&state["fragments"], &regions),
        regions,
        limitations: state
            .get("role_limitations")
            .unwrap_or(&state["limitations"])
            .clone(),
    })
}

pub fn apply(file: &mut FileResult, request: &Value, body: &Value) -> Result<()> {
    let assessment = assess(&request["state"], body)?;
    let regions = &assessment.regions;
    let limits = &assessment.limitations;
    file.status = if regions.iter().any(|r| r["status"] == "needs-context") {
        Status::NeedsContext
    } else if regions.iter().any(|r| r["status"] == "uncertain")
        || limits["regions_omitted"].as_u64().unwrap_or(0) > 0
        || !limits["unsupported_parser_paths"]
            .as_array()
            .unwrap()
            .is_empty()
    {
        Status::Uncertain
    } else {
        Status::Clear
    };
    file.model = body["model"].as_str().map(str::to_owned);
    file.role_assessment = Some(assessment);
    file.dimensions.clear();
    file.file_dimensions.clear();
    file.findings.clear();
    file.syntax_checked = crate::locations::parse(
        &file.path,
        request["state"]["file"]["source"].as_str().unwrap_or(""),
    )?
    .is_some();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{Project, args, run};
    struct Judge;
    impl crate::transport::Evaluator for Judge {
        fn evaluate(&mut self, request: &Value) -> Result<Value> {
            let answers: serde_json::Map<_,_> = request["questions"].as_object().unwrap().keys()
                .map(|key| (key.clone(),json!({"type":"noul","noul":if key.ends_with("application_library") || key.ends_with("evidence") {0.98} else {0.02}}))).collect();
            Ok(
                json!({"model":request["model"],"answers":answers,"usage":{"input_tokens":10,"output_tokens":10}}),
            )
        }
    }
    const MIXED: &str = "fn calculate(value: &str) -> String {\n let name = value.trim().to_lowercase();\n let enabled = !name.is_empty();\n format!(\"{}:{}\", name, enabled)\n}\n#[cfg(test)]\nmod checks {\n #[test]\n fn scenario() {\n let value = \" A \";\n let name = value.trim().to_lowercase();\n let enabled = !name.is_empty();\n assert_eq!(format!(\"{}:{}\", name, enabled), super::calculate(value));\n }\n}\n";
    #[test]
    fn deduplicates_occurrences_and_never_asks_specialist_questions() {
        let p = Project::new();
        p.write("mixed.rs", MIXED);
        let mut options = args();
        options.roles_only = true;
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let q = request(&inputs[0], &options).unwrap();
        let regions = q["state"]["regions"].as_array().unwrap();
        assert!(regions.iter().any(|r| r["name"] == "calculate"));
        assert!(regions.iter().any(|r| r["name"] == "scenario"));
        assert_eq!(q["questions"].as_object().unwrap().len(), regions.len() * 5);
        assert!(
            q["questions"]
                .as_object()
                .unwrap()
                .keys()
                .all(|k| k.starts_with("role_"))
        );
        let source = q["state"]["file"]["source"].as_str().unwrap();
        assert_eq!(source, MIXED);
        let mut identities = BTreeSet::new();
        for region in regions {
            assert!(identities.insert(region.to_string()));
        }
        let mut body = crate::transport::Evaluator::evaluate(&mut Judge, &q).unwrap();
        for (index, region) in regions.iter().enumerate() {
            if region["name"] == "scenario" {
                body["answers"][format!("role_{index}_test_scenario")]["noul"] = json!(0.99);
                body["answers"][format!("role_{index}_application_library")]["noul"] = json!(0.01);
            }
        }
        let mut file = inputs[0].result.clone();
        apply(&mut file, &q, &body).unwrap();
        let assessment = file.role_assessment.unwrap();
        assert!(assessment.relationships.iter().any(|f| {
            f["pairs"].as_array().unwrap().iter().any(|p| {
                p["kinds"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("test-implementation"))
            })
        }));
        assert!(file.dimensions.is_empty() && file.findings.is_empty());
    }
    #[test]
    fn role_cache_isolated_from_maintainability_and_missing_evidence_is_preserved() {
        let p = Project::new();
        p.write(
            "module.py",
            "def transform(value):\n    return value.strip().lower()\n",
        );
        let mut options = args();
        options.roles_only = true;
        let report = run(&p, &options, &mut Judge);
        assert_eq!(report.status, "classified");
        assert_eq!(report.api_requests, 1);
        assert_eq!(run(&p, &options, &mut Judge).api_requests, 0);
        assert_eq!(report.stages["roles"].successful_requests, 1);
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let q = request(&inputs[0], &options).unwrap();
        let mut body = crate::transport::Evaluator::evaluate(&mut Judge, &q).unwrap();
        body["answers"]["role_0_evidence"]["noul"] = json!(0.1);
        let mut file = inputs[0].result.clone();
        apply(&mut file, &q, &body).unwrap();
        assert_eq!(file.status, Status::NeedsContext);
        assert_eq!(
            file.role_assessment.as_ref().unwrap().regions[0]["roles"]["application_library"]["answer"]
                ["noul"],
            0.98
        );
        body["answers"]["role_0_evidence"]["noul"] = json!(0.55);
        apply(&mut file, &q, &body).unwrap();
        assert_eq!(file.status, Status::Uncertain);
        options.roles_only = false;
        options.cache_only = true;
        assert!(!run(&p, &options, &mut Judge).complete);
    }
    #[test]
    fn bounds_unsupported_evidence_and_overlap_do_not_become_clear_relationships() {
        let p = Project::new();
        p.write(
            "many.py",
            &(0..40)
                .map(|i| format!("def f{i}():\n    return {i}\n"))
                .collect::<String>(),
        );
        let mut options = args();
        options.roles_only = true;
        let report = run(&p, &options, &mut Judge);
        let a = report.files[0].role_assessment.as_ref().unwrap();
        assert_eq!(a.regions.len(), LIMIT);
        assert!(a.limitations["regions_omitted"].as_u64().unwrap() > 0);
        assert_eq!(report.files[0].status, Status::Uncertain);
        let region = json!({"status":"uncertain","roles":{"test_scenario":{"status":"present"},"application_library":{"status":"present"}}});
        let relationships = relationships(
            &json!([{"occurrences":[{"region_index":0},{"region_index":0},{"region_index":null}],"locations_omitted":0}]),
            &[region],
        );
        assert_eq!(
            relationships[0]["pairs"][0]["kinds"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(relationships[0]["pairs"][0]["unresolved"], true);
        assert_eq!(relationships[0]["pairs"][1]["unresolved"], true);
        let unsupported = Project::new();
        unsupported.write("module.zig", "pub fn entry() void {}\n");
        options.source_extension = vec!["zig".into()];
        let result = run(&unsupported, &options, &mut Judge);
        assert_eq!(result.files[0].status, Status::Uncertain);
        assert_eq!(
            result.files[0]
                .role_assessment
                .as_ref()
                .unwrap()
                .limitations["unsupported_parser_paths"],
            json!(["module.zig"])
        );
    }
}
