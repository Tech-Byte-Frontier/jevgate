//! A file-scoped maintainability pass. Syntax supplies locations, never verdicts.
use crate::{
    catalog::Rule,
    inventory::Input,
    options::CheckArgs,
    response,
    schema::{Dimension, FileResult, Finding, Status},
};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const KEYS: [&str; 3] = [
    "file_organization",
    "function_simplification",
    "shared_logic",
];
const IDS: [&str; 3] = [
    "maintainability/file-organization",
    "maintainability/function-simplification",
    "maintainability/shared-logic",
];
const QUESTIONS: [&str; 3] = [
    "Classify the primary file's module organization. Does it implement one responsibility, or does it combine independently useful capabilities that would change for different reasons? A responsibility is a separately understandable job with its own concepts, data, and reason to change. A capability is independently useful when it could stand alone as its own module and change without the others. Capabilities that are each useful on their own, such as building inputs, composing judgments, formatting outputs, or follow-up probes, are separate responsibilities even when they serve one product feature. Steps of one workflow and variations of one concern can remain together. Different function names or separate algorithms alone do not establish separate responsibilities. Judge the source's actual concepts and dependencies. `file.line_count` over `file.line_budget` is evidence to weigh when inspecting responsibility boundaries, never a verdict on its own.",
    "Classify the internal organization of the primary file's functions. Is there a function doing several substantial jobs inline, or tangled control flow hiding a coherent task that should be extracted or simplified? Separately understandable parsing, transformation, rendering or delivery phases implemented inline can warrant extraction even in a sequential workflow. A short focused calculation, straightforward guard-and-aggregate operation, delegation to helpers, necessary domain branches and declarative tables are acceptable. The benefit must reduce the reasoning or repeated maintenance needed to change the implementation, not merely shorten it.",
    "Classify implementation repetition involving the primary file. Do supplied operations repeat the same meaningful sequence of implementation steps for the same responsibility, requiring corresponding corrections in multiple places? This includes multi-step setup or cleanup as well as algorithms, validation and transformations. Repeated resource construction, configuration or runtime record assembly is implementation, not merely declarative data. Distinguish shared mechanics from the variable policy values passed into them: consolidating common mechanics need not merge the policies. Once those mechanics are delegated, repeated calls with different policy inputs are acceptable. Tiny constructors and forwarding wrappers do not by themselves warrant consolidation. In tests, distinguish substantial repeated fixture implementation from short independent examples and assertion scaffolding. Or is their similarity incidental, trivial, already delegated, declarative, or part of independently owned policies? Shared implementation is useful when it removes repeated maintenance without coupling different meanings. Similar control-flow shape or thresholds alone is not duplicated responsibility. Compare only supplied source.",
];
const CONCERNS: [&str; 3] = [
    "The file combines two or more capabilities that could each stand alone as a module and change for different reasons; separate modules would give useful responsibility boundaries.",
    "A function implements multiple substantial tasks inline or has unnecessarily tangled control flow; extracting a coherent task or restructuring it would make changes easier to understand.",
    "The same meaningful algorithm, transformation, validation, setup, cleanup or runtime assembly sequence is repeated for the same responsibility; a shared helper or fixture would remove corresponding maintenance edits.",
];
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Assessment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cascade: Option<crate::cascade::Comparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_operation: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub operations: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_fragment: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fragments: BTreeMap<String, Value>,
    pub outcome: Value,
    pub location: Value,
    pub selected_locations: Vec<Value>,
}

pub fn policy() -> BTreeMap<&'static str, f64> {
    BTreeMap::from([
        ("review_probability", response::REVIEW_PROBABILITY),
        ("clear_probability", response::REVIEW_PROBABILITY),
        ("location_probability", response::LOCATION_PROBABILITY),
        ("missing_context", response::MISSING_CONTEXT),
        ("extract_confidence", EXTRACT_CONFIDENCE),
        ("line_budget", crate::options::DEFAULT_LINE_BUDGET as f64),
    ])
}

pub fn rules() -> Vec<Rule> {
    KEYS.iter()
        .enumerate()
        .map(|(i, key)| Rule {
            id: IDS[i],
            key,
            version: crate::catalog::rule_version(key),
            scope: "selected complete file and explicit supporting files",
            required_context: "Complete selected source; explicit peers when needed",
            inspection: QUESTIONS[i],
            acceptable_example: "Cohesive code with useful existing boundaries",
            evaluation_dataset: "focused development controls; not calibrated",
            thresholds_validated: false,
        })
        .collect()
}
pub fn is_rule(s: &str) -> bool {
    KEYS.contains(&s) || IDS.contains(&s)
}
#[cfg(test)]
pub fn request(input: &Input, args: &CheckArgs) -> Result<Value> {
    let view = crate::file_kind::gate_view(input, args)?;
    request_with(input, args, &view)
}

pub(crate) fn request_with(
    input: &Input,
    args: &CheckArgs,
    view: &crate::file_kind::View,
) -> Result<Value> {
    let source = view.source.as_str();
    let scope = crate::file_kind::scope_sentence(&view.classification);
    // Fail on invalid supported syntax, rather than treating absent candidates as clear.
    crate::locations::parse(&input.result.path, source)?;
    let mut operations = Vec::new();
    for (path, text) in std::iter::once((&input.result.path, source)).chain(
        input
            .context
            .iter()
            .map(|c| (&c.file.path, c.source.as_str())),
    ) {
        if path != &input.result.path || operations.is_empty() {
            let callable_lines = crate::locations::callable_lines(path, text)?;
            for (name, range, _) in crate::context_units::review_targets(path, text) {
                let callable = callable_lines
                    .iter()
                    .any(|line| (range.start_line..=range.end_line).contains(line));
                operations.push(json!({"path":path,"name":name,"range":range,"callable":callable}));
            }
        }
    }
    let total = operations.len();
    operations.truncate(64);
    let primary = |v: &Value| v["path"] == json!(input.result.path);
    let mut pairs = Vec::new();
    let mut pair_count: usize = 0;
    for i in 0..operations.len() {
        for j in i + 1..operations.len() {
            if primary(&operations[i]) || primary(&operations[j]) {
                pair_count += 1;
                if pairs.len() < 240 {
                    pairs.push(json!([i, j]));
                }
            }
        }
    }
    let mut questions = serde_json::Map::new();
    for (index, key) in KEYS.iter().enumerate() {
        if !args.rules.iter().any(|r| r == key || r == IDS[index]) {
            continue;
        }
        let mut criteria = serde_json::Map::from_iter([(
            "none".into(),
            json!(
                "No supplied location expresses a concrete refactoring boundary; the relevant location is absent or the question does not apply."
            ),
        )]);
        if index == 1 {
            for (i, op) in operations
                .iter()
                .enumerate()
                .filter(|(_, op)| primary(op) && op["callable"] == true)
            {
                criteria.insert(
                    format!("o{i}"),
                    json!(format!(
                        "A concrete internal organization problem warrants simplifying operations[{i}] ({}); the benefit goes beyond merely being able to extract lines.",
                        op["name"]
                    )),
                );
            }
        } else {
            for (i, pair) in pairs.iter().enumerate() {
                let a = pair[0].as_u64().unwrap() as usize;
                let b = pair[1].as_u64().unwrap() as usize;
                if index == 0 && !(primary(&operations[a]) && primary(&operations[b])) {
                    continue;
                }
                if index == 2
                    && !(operations[a]["callable"] == true && operations[b]["callable"] == true)
                {
                    continue;
                }
                criteria.insert(
                    format!("p{i}"),
                    json!(format!("Operations {a} and {b} identified by pairs[{i}].")),
                );
            }
        }
        questions.insert((*key).into(), crate::questions::verdict(index, &scope));
        if index == 0 {
            criteria.insert("none".into(),json!("No candidate pair represents different responsibilities, or the relevant operations are absent from the candidates."));
        }
        questions.insert(
            format!("{key}_location"),
            json!({"type":"choice",
            "instructions":crate::questions::location_instructions(index),
            "criteria":criteria}),
        );
    }
    if questions.contains_key("function_simplification") {
        for (index, operation) in operations
            .iter()
            .enumerate()
            .filter(|(_, op)| primary(op) && op["callable"] == true)
        {
            let start = operation["range"]["start_line"].as_u64().unwrap_or(1) as usize;
            let end = operation["range"]["end_line"]
                .as_u64()
                .unwrap_or(start as u64) as usize;
            let excerpt = source
                .lines()
                .skip(start.saturating_sub(1))
                .take(end.saturating_sub(start) + 1)
                .collect::<Vec<_>>()
                .join("\n");
            questions.insert(
                format!("operation_probe_{index}"),
                crate::questions::operation_probe(operation, &excerpt),
            );
        }
    }
    let mut repetition = crate::repetition::observations(
        std::iter::once((input.result.path.as_path(), source)).chain(
            input
                .context
                .iter()
                .map(|c| (c.file.path.as_path(), c.source.as_str())),
        ),
    )?;
    let mut excerpt_bytes = 4096;
    let mut excerpts_omitted = 0;
    for fragment in repetition["observations"].as_array_mut().unwrap() {
        fragment["surroundings"] = fragment_surroundings(
            input,
            source,
            fragment,
            &mut excerpt_bytes,
            &mut excerpts_omitted,
        );
    }
    if questions.contains_key("shared_logic") {
        for index in 0..repetition["observations"].as_array().unwrap().len() {
            questions.insert(
                format!("shared_logic_fragment_{index}"),
                crate::questions::fragment(index),
            );
        }
    }
    let cascade_enabled = questions.contains_key("shared_logic");
    let line_count = input.source.as_deref().unwrap_or(source).lines().count() as u64;
    let mut request = json!({"model":args.model,"state":{
        "maintainability_version":9,
        "file":{"path":input.result.path,"role":input.result.role,"language":view.classification.language,"classification":crate::file_kind::model_value(&view.classification),"source":source,"source_hash":input.result.source_hash,"line_count":line_count,"line_budget":args.line_budget},
        "context":input.context.iter().map(|c| json!({"path":c.file.path,"source":c.source,"source_hash":c.file.source_hash})).collect::<Vec<_>>(),
        "operations":operations,"pairs":pairs,"repeated_fragments":repetition["observations"],
        "limitations":{"fragment_excerpts_omitted":excerpts_omitted,"operations_omitted":total.saturating_sub(64),"pairs_omitted":pair_count.saturating_sub(240),"scope":"Selected file and explicit context only. Nested tasks and non-callable declarations may lack separate candidates. Full source remains visible; use context for an unrepresentable important opportunity."}
    },"questions":questions});
    if cascade_enabled {
        let judged = Input {
            result: input.result.clone(),
            source: Some(source.to_string()),
            context: input.context.clone(),
        };
        crate::cascade::attach(
            &mut request,
            crate::roles::occurrence_request(&judged, args)?,
        )?;
    }
    Ok(request)
}

// Optional location hints, not verdicts or substitutes for the complete source.
// Share them once per request and bound their extra size across all fragments.
fn fragment_surroundings(
    input: &Input,
    primary: &str,
    fragment: &Value,
    remaining: &mut usize,
    omitted: &mut usize,
) -> Value {
    let mut excerpts = Vec::new();
    for location in fragment["locations"].as_array().unwrap() {
        let path = location["path"].as_str().unwrap();
        let source = if path == input.result.path.to_string_lossy() {
            Some(primary)
        } else {
            input
                .context
                .iter()
                .find(|c| c.file.path.to_string_lossy() == path)
                .map(|c| c.source.as_str())
        };
        let Some(source) = source else {
            *omitted += 1;
            continue;
        };
        let start = location["start_line"].as_u64().unwrap().saturating_sub(7) as usize;
        let end = location["end_line"].as_u64().unwrap() as usize + 6;
        let excerpt = json!({"path":path,"start_line":start + 1,"source":source.lines().skip(start).take(end - start).collect::<Vec<_>>().join("\n")});
        let bytes = serde_json::to_vec(&excerpt).unwrap().len();
        if bytes <= *remaining {
            *remaining -= bytes;
            excerpts.push(excerpt);
        } else {
            *omitted += 1;
        }
    }
    json!(excerpts)
}

pub fn apply(file: &mut FileResult, request: &Value, body: &Value) -> Result<()> {
    file.dimensions.clear();
    file.file_dimensions.clear();
    file.findings.clear();
    file.context_requests.clear();
    file.model = body["model"].as_str().map(str::to_owned);
    file.role_assessment = request["state"]["cascade_role_version"]
        .is_string()
        .then(|| crate::roles::assess(&request["state"], body))
        .transpose()?;
    let limits = &request["state"]["limitations"];
    let scope = format!(
        "Maintainability scope: selected file and explicit context only; {} operations and {} pairs omitted. Nested tasks and non-callable declarations may lack candidates; shared logic is not a repository-wide clone scan. Up to 12 repeated token fragments are judged, with at most 20 locations each; absence of a fragment is not proof of no duplication.",
        limits["operations_omitted"], limits["pairs_omitted"]
    );
    let scope = format!(
        "{scope} {} optional fragment excerpts omitted by the 4096-byte shared excerpt budget; full source and fragment locations remain available.",
        limits["fragment_excerpts_omitted"]
    );
    let scope = if limits["operation_excerpts_omitted"].as_u64().unwrap_or(0) > 0
        || limits["operation_probes_omitted"].as_u64().unwrap_or(0) > 0
    {
        format!(
            "{scope} {} operation probes omit a duplicated excerpt and {} probes were dropped to stay within the provider budget. The complete file.source remains, and absence of a probe is not a clear judgment.",
            limits["operation_excerpts_omitted"].as_u64().unwrap_or(0),
            limits["operation_probes_omitted"].as_u64().unwrap_or(0)
        )
    } else {
        scope
    };
    if !file.context_limitations.contains(&scope) {
        file.context_limitations.push(scope);
    }
    file.syntax_checked = crate::locations::parse(
        &file.path,
        request["state"]["file"]["source"].as_str().unwrap_or(""),
    )
    .is_ok_and(|tree| tree.is_some());
    for (key, answer) in body["answers"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(key, _)| KEYS.contains(&key.as_str()))
    {
        let probabilities: BTreeMap<String, f64> =
            serde_json::from_value(answer["probabilities"].clone())?;
        let mass: f64 = probabilities.values().sum();
        let concern = probabilities.get("review").copied().unwrap_or(0.0) / mass;
        let missing = probabilities.get("context").copied().unwrap_or(0.0) / mass;
        let clear = probabilities.get("clear").copied().unwrap_or(0.0) / mass;
        let na = probabilities.get("not_applicable").copied().unwrap_or(0.0) / mass;
        let mut status = if response::probability_at_least(concern, response::REVIEW_PROBABILITY) {
            Status::Review
        } else if response::probability_at_least(clear, response::REVIEW_PROBABILITY) {
            Status::Clear
        } else if response::probability_at_least(na, response::REVIEW_PROBABILITY) {
            Status::NotApplicable
        } else if missing >= response::MISSING_CONTEXT {
            Status::NeedsContext
        } else {
            Status::Uncertain
        };
        let location = &body["answers"][format!("{key}_location")];
        let location_probabilities: BTreeMap<String, f64> =
            serde_json::from_value(location["probabilities"].clone())?;
        let best = location_probabilities
            .iter()
            .filter(|(k, _)| k.starts_with('o') || k.starts_with('p'))
            .max_by(|a, b| a.1.total_cmp(b.1));
        let mut selected_locations = Vec::new();
        let mut detail = match status {
            Status::Clear => "No concrete beneficial change established in the supplied scope.",
            Status::NotApplicable => "No applicable operations in this scope.",
            Status::NeedsContext => {
                "Supply the missing implementation or a more focused candidate boundary."
            }
            _ => "No decisive judgment; inspect the raw probabilities.",
        }
        .to_string();
        if let Some((choice, probability)) =
            best.filter(|(_, p)| **p > 0.0 && location["choice"] != "none")
        {
            let selected: Vec<&Value> = if let Some(i) = choice
                .strip_prefix('o')
                .and_then(|s| s.parse::<usize>().ok())
            {
                vec![&request["state"]["operations"][i]]
            } else {
                let i = choice[1..].parse::<usize>()?;
                request["state"]["pairs"][i]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| &request["state"]["operations"][v.as_u64().unwrap() as usize])
                    .collect()
            };
            selected_locations = selected.iter().map(|v| (*v).clone()).collect();
            let names = selected
                .iter()
                .map(|v| {
                    format!(
                        "{}:{}–{} ({})",
                        v["path"].as_str().unwrap_or(""),
                        v["range"]["start_line"],
                        v["range"]["end_line"],
                        v["name"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join(" and ");
            let action = match key.as_str() {
                "file_organization" => {
                    "Consider separate modules for independently changing responsibilities"
                }
                "function_simplification" => {
                    "Inspect this function for a coherent task to extract or control flow to simplify"
                }
                _ => {
                    "Consider a shared implementation for repeated logic that should change together"
                }
            };
            if status == Status::Uncertain {
                detail = format!(
                    "No decisive judgment. Conditional candidate: {names} (probability {probability:.2}); this does not establish a need to refactor."
                );
            }
            let located =
                response::probability_at_least(*probability, response::LOCATION_PROBABILITY);
            if status == Status::Review && !located {
                if key == "shared_logic" {
                    detail = format!("Review without a supported location ({probability:.2}).");
                    status = Status::Uncertain;
                } else {
                    // Location is an advisory boundary, not a verdict. A weak
                    // pair neither erases a decisive review nor invents a split.
                    selected_locations.clear();
                }
            }
            if status == Status::Review && located {
                detail = if key == "shared_logic" {
                    pair_detail(&names, &selected, request)
                } else {
                    format!(
                        "{action}: {names}. Strongest candidate probability {probability:.2}; this is an advisory boundary, not a generated refactoring plan."
                    )
                };
                let primary = selected
                    .iter()
                    .find(|v| v["path"] == json!(file.path))
                    .unwrap_or(&selected[0]);
                file.findings.push(Finding {
                    rule: rules()
                        .into_iter()
                        .find(|r| r.key == key)
                        .unwrap()
                        .id
                        .into(),
                    line: primary["range"]["start_line"].as_u64().unwrap_or(1) as usize,
                    message: detail.clone(),
                    action: action.into(),
                    symbol: Some(names),
                    rule_version: crate::catalog::rule_version(key).into(),
                    concern_probability: concern,
                    evidence_complete: missing < response::MISSING_CONTEXT,
                });
            }
        }
        if status == Status::Review && selected_locations.is_empty() {
            if key == "shared_logic" {
                detail = "Shared-logic concern without a supplied pair. A focused follow-up asks which pair repeats.".into();
                status = Status::Uncertain;
            } else {
                detail = "Refactoring concern identified, but no supplied candidate localizes its boundary. Review the complete file; do not infer a specific split.".into();
                file.findings.push(Finding {
                    rule: rules()
                        .into_iter()
                        .find(|r| r.key == key)
                        .unwrap()
                        .id
                        .into(),
                    line: 1,
                    message: detail.clone(),
                    action: "Inspect the file to identify a concrete refactoring boundary".into(),
                    symbol: None,
                    rule_version: crate::catalog::rule_version(key).into(),
                    concern_probability: concern,
                    evidence_complete: false,
                });
            }
        }
        let dimension = Dimension {
            refactoring_assessment: Some(Assessment {
                cascade: (key == "shared_logic")
                    .then(|| crate::cascade::compare(request, body, file.role_assessment.as_ref()))
                    .flatten(),
                selected_operation: None,
                operations: body["answers"].as_object().unwrap().iter()
                    .filter(|(name, _)| key == "function_simplification" && name.starts_with("operation_probe_"))
                    .map(|(name, answer)| (name.clone(), json!({"answer":answer,"evidence":request["questions"][name]["instructions"]["operation"]}))).collect(),
                selected_fragment: None,
                fragments: body["answers"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .filter(|(name, _)| {
                        key == "shared_logic" && name.starts_with("shared_logic_fragment_")
                    })
                    .map(|(name, answer)| (name.clone(), json!({"answer":answer,"evidence":request["state"]["repeated_fragments"][request["questions"][name]["instructions"]["fragment_index"].as_u64().unwrap() as usize]})))
                    .collect(),
                outcome: answer.clone(),
                location: location.clone(),
                selected_locations,
            }),

            score: 0.0,
            confidence: answer["confidence"].as_f64().unwrap_or(0.0),
            probabilities,
            concern_probability: concern,
            concern_basis: "direct-maintainability-outcome".into(),
            missing_context: missing,
            evidence_sufficiency: Some(1.0 - missing),
            protection_probability: None,
            protection_checks: Default::default(),
            decision_basis: detail,
            status,
            rule_version: crate::catalog::rule_version(key).into(),
            applicability: "Selected source and explicit context only".into(),
            applicability_probability: None,
        };
        file.dimensions.insert(key.clone(), dimension.clone());
        file.file_dimensions.insert(key.clone(), dimension);
    }
    apply_operations(file)?;
    apply_fragments(file)?;
    response::update_status(file);
    Ok(())
}

/// One follow-up for functions that already scored review. The choice is the
/// task to extract, or none when the length is the work itself.
pub fn extraction_requests(
    input: &Input,
    file: &FileResult,
    args: &CheckArgs,
) -> Result<Vec<Value>> {
    let Some(source) = input.source.as_deref() else {
        return Ok(Vec::new());
    };
    let Some(dimension) = file.dimensions.get("function_simplification") else {
        return Ok(Vec::new());
    };
    let Some(assessment) = dimension.refactoring_assessment.as_ref() else {
        return Ok(Vec::new());
    };
    let mut hot: Vec<_> = assessment
        .operations
        .values()
        .filter_map(|operation| {
            let probabilities = operation["answer"]["probabilities"].as_object()?;
            let concern = concern_of(probabilities, &["review"])?;
            response::probability_at_least(concern, response::REVIEW_PROBABILITY)
                .then_some((operation, concern))
        })
        .collect();
    hot.sort_by(|left, right| right.1.total_cmp(&left.1));
    let omitted = hot.len().saturating_sub(EXTRACTION_LIMIT);
    hot.truncate(EXTRACTION_LIMIT);
    if hot.is_empty() {
        return Ok(Vec::new());
    }
    let mut questions = serde_json::Map::new();
    for (index, (operation, _)) in hot.iter().enumerate() {
        let evidence = &operation["evidence"];
        questions.insert(
            format!("function_simplification_extract_{index}"),
            crate::questions::extraction(evidence, &operation_excerpt(source, evidence)),
        );
    }
    Ok(vec![json!({
        "model": args.model,
        "state": {
            "extraction_version": 1,
            "file": {"path": file.path, "source_hash": file.source_hash},
            "limitations": {"extraction_omitted": omitted}
        },
        "questions": questions
    })])
}

pub fn apply_extraction(file: &mut FileResult, request: &Value, body: &Value) -> Result<()> {
    {
        let Some(dimension) = file.dimensions.get_mut("function_simplification") else {
            return Ok(());
        };
        let Some(assessment) = dimension.refactoring_assessment.as_mut() else {
            return Ok(());
        };
        let questions = request["questions"]
            .as_object()
            .context("Missing questions")?;
        let answers = body["answers"].as_object().context("Missing answers")?;
        for (name, question) in questions {
            if !name.starts_with("function_simplification_extract_") {
                continue;
            }
            let Some(answer) = answers.get(name) else {
                continue;
            };
            let probe = &question["instructions"]["operation"];
            for operation in assessment.operations.values_mut() {
                if operation["evidence"] == *probe {
                    operation["extraction"] = answer.clone();
                }
            }
        }
    }
    apply_operations(file)
}

/// One follow-up per uncertain dimension. The state is only the undecided
/// operation or repeated lines, or the file alone when the uncertain question
/// is file organization. The question text matches the first pass.
pub fn focused_requests(input: &Input, file: &FileResult, args: &CheckArgs) -> Result<Vec<Value>> {
    let Some(original) = input.source.as_deref() else {
        return Ok(Vec::new());
    };
    let source = crate::file_kind::judgment_source(original, file.classification.as_ref());
    let mut requests = Vec::new();
    for (index, key) in KEYS.iter().enumerate() {
        let Some(dimension) = file.dimensions.get(*key) else {
            continue;
        };
        if dimension.status != Status::Uncertain {
            continue;
        }
        // A decisive file-wide verdict is already a gate answer. Weak or missing
        // localization is reported as uncertain and is not re-asked; Jev already
        // said review or clear on the file.
        let probabilities = &dimension.probabilities;
        let review = probabilities.get("review").copied().unwrap_or(0.0);
        let clear = probabilities.get("clear").copied().unwrap_or(0.0);
        if response::probability_at_least(review, response::REVIEW_PROBABILITY)
            || response::probability_at_least(clear, response::REVIEW_PROBABILITY)
        {
            continue;
        }
        let Some(excerpt) = focus_excerpt(&source, input, key, dimension) else {
            continue;
        };
        let mut questions = serde_json::Map::new();
        questions.insert(
            (*key).into(),
            verdict_question(index, file.classification.as_ref()),
        );
        let mut location_candidates = Vec::new();
        if *key == "shared_logic" {
            let (criteria, candidates) = shared_location_criteria(input, dimension);
            if !candidates.is_empty() {
                questions.insert(
                    "shared_logic_location".into(),
                    json!({
                        "type": "choice",
                        "instructions": crate::questions::location_instructions(2),
                        "criteria": criteria,
                    }),
                );
                location_candidates = candidates;
            }
        }
        requests.push(json!({
            "model": args.model,
            "state": {
                "focused": key,
                "file": {
                    "path": input.result.path,
                    "source": excerpt.text,
                    "classification": file.classification.as_ref().map(crate::file_kind::model_value)
                },
                "focus_evidence": excerpt.evidence,
                "location_candidates": location_candidates
            },
            "questions": questions
        }));
    }
    Ok(requests)
}

fn shared_location_criteria(
    input: &Input,
    dimension: &Dimension,
) -> (serde_json::Map<String, Value>, Vec<Value>) {
    let mut criteria = serde_json::Map::new();
    let mut candidates = Vec::new();
    criteria.insert(
        "none".into(),
        json!("No supplied pair repeats implementation for the same responsibility."),
    );
    let Some(source) = input.source.as_deref() else {
        return (criteria, candidates);
    };
    let mut index = 0usize;
    if let Some(assessment) = dimension.refactoring_assessment.as_ref() {
        // Prefer the first-pass pair when one was already selected, even if the
        // location share was below the bar. The follow-up can confirm or reject it.
        if !assessment.selected_locations.is_empty() {
            let names = assessment
                .selected_locations
                .iter()
                .map(|location| {
                    location_label(
                        location["path"].as_str().unwrap_or(""),
                        location["range"]["start_line"].as_u64().unwrap_or(1),
                        location["range"]["end_line"].as_u64().unwrap_or(1),
                    )
                })
                .collect::<Vec<_>>()
                .join(" and ");
            let copies: Vec<Value> = assessment
                .selected_locations
                .iter()
                .map(|location| {
                    let start = location["range"]["start_line"].as_u64().unwrap_or(1) as usize;
                    let end = location["range"]["end_line"]
                        .as_u64()
                        .unwrap_or(start as u64) as usize;
                    json!({
                        "path": location["path"],
                        "start_line": start,
                        "source": clip_text(&line_window(source, start, end)),
                    })
                })
                .collect();
            let repeated = copies
                .first()
                .and_then(|copy| copy["source"].as_str())
                .unwrap_or("")
                .to_string();
            criteria.insert(
                format!("p{index}"),
                json!(format!(
                    "Operations {names} repeat the same implementation."
                )),
            );
            candidates.push(json!({
                "key": format!("p{index}"),
                "names": names,
                "locations": assessment.selected_locations,
                "evidence": {"source": clip_text(&repeated), "surroundings": copies},
            }));
            index += 1;
        }
        for fragment in assessment.fragments.values() {
            let Some(locations) = fragment["evidence"]["locations"].as_array() else {
                continue;
            };
            let names = locations
                .iter()
                .map(|location| {
                    location_label(
                        location["path"].as_str().unwrap_or(""),
                        location["start_line"].as_u64().unwrap_or(1),
                        location["end_line"].as_u64().unwrap_or(1),
                    )
                })
                .collect::<Vec<_>>()
                .join(" and ");
            if names.is_empty() {
                continue;
            }
            criteria.insert(
                format!("p{index}"),
                json!(format!(
                    "Operations {names} repeat the same implementation."
                )),
            );
            candidates.push(json!({
                "key": format!("p{index}"),
                "names": names,
                "locations": locations,
                "evidence": {
                    "source": clip_text(fragment["evidence"]["source"].as_str().unwrap_or("")),
                    "surroundings": fragment["evidence"]["surroundings"].clone(),
                },
            }));
            index += 1;
        }
    }
    if index == 0 {
        for (name, range, _) in crate::context_units::review_targets(&input.result.path, source)
            .iter()
            .take(16)
        {
            criteria.insert(
                format!("o{index}"),
                json!(format!(
                    "Operation {name} at {}:{}–{} embodies the repeated implementation.",
                    input.result.path.display(),
                    range.start_line,
                    range.end_line
                )),
            );
            candidates.push(json!({
                "key": format!("o{index}"),
                "names": name,
                "locations": [{
                    "path": input.result.path,
                    "start_line": range.start_line,
                    "end_line": range.end_line,
                }],
                "evidence": {
                    "source": clip_text(&line_window(source, range.start_line, range.end_line)),
                    "surroundings": [{
                        "path": input.result.path,
                        "start_line": range.start_line,
                        "source": clip_text(&line_window(source, range.start_line, range.end_line)),
                    }],
                },
            }));
            index += 1;
        }
    }
    (criteria, candidates)
}

struct FocusExcerpt {
    text: String,
    evidence: Vec<Value>,
}

fn focus_excerpt(
    source: &str,
    input: &Input,
    key: &str,
    dimension: &Dimension,
) -> Option<FocusExcerpt> {
    let assessment = dimension.refactoring_assessment.as_ref()?;
    if key == "function_simplification" {
        let operations = selected_children(&assessment.operations, false);
        return Some(excerpt_from_ranges(source, input, &operations));
    }
    if key == "shared_logic" {
        let fragments = selected_children(&assessment.fragments, true);
        return Some(excerpt_from_ranges(source, input, &fragments));
    }
    Some(FocusExcerpt {
        text: source.to_string(),
        evidence: vec![
            json!({"name": "file", "start_line": 1, "end_line": source.lines().count()}),
        ],
    })
}

fn selected_children(children: &BTreeMap<String, Value>, fragment: bool) -> Vec<Value> {
    let undecided: Vec<Value> = children
        .values()
        .filter(|child| !choice_is_decisive(&child["answer"]["probabilities"], fragment))
        .map(|child| child["evidence"].clone())
        .collect();
    if undecided.is_empty() {
        children
            .values()
            .map(|child| child["evidence"].clone())
            .collect()
    } else {
        undecided
    }
}

fn choice_is_decisive(probabilities: &Value, fragment: bool) -> bool {
    let Some(probabilities) = probabilities.as_object() else {
        return false;
    };
    let total: f64 = probabilities.values().filter_map(Value::as_f64).sum();
    if total <= 0.0 {
        return false;
    }
    let review = if fragment {
        ["shared_setup", "shared_construction", "shared_algorithm"]
            .iter()
            .map(|key| {
                probabilities
                    .get(*key)
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0)
            })
            .sum::<f64>()
    } else {
        probabilities
            .get("review")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    let clear = if fragment {
        [
            "independent_policy",
            "delegated",
            "intentional_sequence",
            "idiom",
            "data",
        ]
        .iter()
        .map(|key| {
            probabilities
                .get(*key)
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        })
        .sum::<f64>()
    } else {
        probabilities
            .get("clear")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
    };
    let absent = probabilities
        .get("not_applicable")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    response::probability_at_least(review / total, response::REVIEW_PROBABILITY)
        || response::probability_at_least(clear / total, response::REVIEW_PROBABILITY)
        || response::probability_at_least(absent / total, response::REVIEW_PROBABILITY)
}

fn excerpt_from_ranges(source: &str, input: &Input, evidence: &[Value]) -> FocusExcerpt {
    let mut text = String::new();
    let mut kept = Vec::new();
    for item in evidence {
        let ranges = if item["locations"].is_array() {
            item["locations"].as_array().cloned().unwrap_or_default()
        } else if item["range"].is_object() {
            vec![json!({
                "path": item["path"],
                "start_line": item["range"]["start_line"],
                "end_line": item["range"]["end_line"],
                "name": item["name"]
            })]
        } else {
            continue;
        };
        for range in ranges {
            let path = range["path"].as_str().unwrap_or("");
            let start = range["start_line"].as_u64().unwrap_or(1) as usize;
            let end = range["end_line"].as_u64().unwrap_or(start as u64) as usize;
            let owner = if path == input.result.path.to_string_lossy() {
                Some(source)
            } else {
                input
                    .context
                    .iter()
                    .find(|context| context.file.path.to_string_lossy() == path)
                    .map(|context| context.source.as_str())
            };
            let Some(owner) = owner else {
                continue;
            };
            let slice = line_window(owner, start, end);
            if text.len() + slice.len() > 24_000 {
                break;
            }
            let name = range["name"]
                .as_str()
                .or_else(|| item["name"].as_str())
                .unwrap_or("excerpt");
            text.push_str(&format!("[{name} lines {start}–{end}]\n{slice}\n\n"));
            let mut entry =
                json!({"name": name, "path": path, "start_line": start, "end_line": end});
            if item["locations"].is_array() {
                entry["locations"] = item["locations"].clone();
            }
            if item["source"].is_string() {
                entry["source"] = item["source"].clone();
            }
            if item["surroundings"].is_array() {
                entry["surroundings"] = item["surroundings"].clone();
            }
            kept.push(entry);
        }
    }
    if text.is_empty() {
        text = source.to_string();
        kept.push(json!({"name": "file", "start_line": 1, "end_line": source.lines().count()}));
    }
    FocusExcerpt {
        text,
        evidence: kept,
    }
}

fn line_window(source: &str, start: usize, end: usize) -> String {
    let start = start.max(1);
    source
        .lines()
        .skip(start - 1)
        .take(end.saturating_sub(start) + 1)
        .collect::<Vec<_>>()
        .join("\n")
}

fn verdict_question(index: usize, class: Option<&crate::file_kind::Classification>) -> Value {
    crate::questions::recheck(index, &crate::file_kind::focus_note(class))
}

pub fn apply_focused(file: &mut FileResult, request: &Value, body: &Value) -> Result<()> {
    let Some(key) = request["state"]["focused"].as_str() else {
        return Ok(());
    };
    let Some(dimension) = file.dimensions.get(key) else {
        return Ok(());
    };
    if dimension.status != Status::Uncertain {
        return Ok(());
    }
    let answer = &body["answers"][key];
    let probabilities: BTreeMap<String, f64> =
        serde_json::from_value(answer["probabilities"].clone()).unwrap_or_default();
    let mass: f64 = probabilities.values().sum();
    if mass <= 0.0 {
        return Ok(());
    }
    let concern = probabilities.get("review").copied().unwrap_or(0.0) / mass;
    let clear = probabilities.get("clear").copied().unwrap_or(0.0) / mass;
    let mut status = if response::probability_at_least(concern, response::REVIEW_PROBABILITY) {
        Status::Review
    } else if response::probability_at_least(clear, response::REVIEW_PROBABILITY) {
        Status::Clear
    } else {
        return Ok(());
    };
    let evidence = request["state"]["focus_evidence"]
        .as_array()
        .and_then(|items| items.first());
    let line = evidence
        .and_then(|item| item["start_line"].as_u64())
        .unwrap_or(1) as usize;
    let name = evidence
        .and_then(|item| item["name"].as_str())
        .unwrap_or("the filtered source");
    let index = KEYS
        .iter()
        .position(|candidate| *candidate == key)
        .unwrap_or(0);
    let mut detail = match status {
        Status::Review => {
            if key == "shared_logic" {
                let Some(item) = evidence else {
                    return Ok(());
                };
                let span = fragment_span(item);
                if span < MIN_SHARED_SPAN {
                    let dimension = file.dimensions.get_mut(key).unwrap();
                    dimension.decision_basis = format!(
                        "{}\nShort repeated span: {span} bytes stayed a probability ({concern:.2}) and was not printed as a finding.",
                        overview_basis(&dimension.decision_basis)
                    );
                    file.findings.retain(|finding| {
                        finding.rule != IDS[2] || finding.message.starts_with("Test portion:")
                    });
                    response::update_status(file);
                    return Ok(());
                }
                let names = if let Some(locations) = item["locations"].as_array() {
                    locations
                        .iter()
                        .map(|location| {
                            location_label(
                                location["path"].as_str().unwrap_or(""),
                                location["start_line"].as_u64().unwrap_or(1),
                                location["end_line"].as_u64().unwrap_or(1),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" and ")
                } else {
                    location_label(
                        item["path"].as_str().unwrap_or(""),
                        item["start_line"].as_u64().unwrap_or(1),
                        item["end_line"].as_u64().unwrap_or(1),
                    )
                };
                shared_detail(&names, item)
            } else {
                format!(
                    "Focused recheck of {name} found a maintainability concern. {}",
                    CONCERNS[index]
                )
            }
        }
        _ => format!(
            "Focused recheck of {name} found no concrete maintainability benefit. The first-pass overview stayed uncertain."
        ),
    };
    let rule = IDS[index];
    let focused_location = &body["answers"]["shared_logic_location"];
    let location_choice = focused_location["choice"].as_str().unwrap_or("none");
    let location_probability = focused_location["probabilities"]
        .as_object()
        .and_then(|probs| probs.get(location_choice))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let selected_candidate = request["state"]["location_candidates"]
        .as_array()
        .and_then(|candidates| {
            candidates
                .iter()
                .find(|candidate| candidate["key"] == json!(location_choice))
        })
        .cloned();
    {
        let dimension = file.dimensions.get_mut(key).unwrap();
        dimension.status = status.clone();
        dimension.probabilities = probabilities;
        dimension.concern_probability = concern;
        dimension.confidence = answer["confidence"].as_f64().unwrap_or(0.0);
        dimension.concern_basis = "focused-evidence".into();
        dimension.decision_basis = detail.clone();
        if key == "shared_logic"
            && status == Status::Review
            && request["questions"].get("shared_logic_location").is_some()
        {
            let located = location_choice != "none"
                && response::probability_at_least(
                    location_probability,
                    response::LOCATION_PROBABILITY,
                )
                && selected_candidate.is_some();
            if located {
                let candidate = selected_candidate.unwrap();
                if let Some(assessment) = dimension.refactoring_assessment.as_mut() {
                    assessment.selected_locations = candidate["locations"]
                        .as_array()
                        .map(|locations| {
                            locations
                                .iter()
                                .map(|location| {
                                    json!({"path":location["path"],"range":{"start_line":location["start_line"],"end_line":location["end_line"]},"name":"repeated source fragment"})
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                }
                let names = candidate["names"].as_str().unwrap_or(name);
                detail = shared_detail(names, &candidate["evidence"]);
                dimension.decision_basis = detail.clone();
            } else {
                status = Status::Uncertain;
                dimension.status = Status::Uncertain;
                detail = if location_choice == "none" {
                    "Shared-logic concern without a supplied pair. A focused follow-up asked which pair repeats and none fit.".into()
                } else {
                    format!("Review without a supported location ({location_probability:.2}).")
                };
                dimension.decision_basis = detail.clone();
            }
        }
    }
    // Test-portion findings share the rule id and stay beside an application recheck.
    file.findings
        .retain(|finding| finding.rule != rule || finding.message.starts_with("Test portion:"));
    if status == Status::Review {
        file.findings.push(Finding {
            rule: rule.into(),
            line,
            message: detail,
            action: "Inspect the filtered evidence before changing it.".into(),
            symbol: Some(name.into()),
            rule_version: crate::catalog::rule_version(key).into(),
            concern_probability: concern,
            evidence_complete: true,
        });
    }
    response::update_status(file);
    Ok(())
}

pub fn apply_test_portion(file: &mut FileResult, request: &Value, body: &Value) -> Result<()> {
    let dimensions = std::mem::take(&mut file.dimensions);
    let file_dimensions = std::mem::take(&mut file.file_dimensions);
    let findings = std::mem::take(&mut file.findings);
    let limitations = std::mem::take(&mut file.context_limitations);
    let role = file.role_assessment.clone();
    let syntax = file.syntax_checked;
    let model = file.model.clone();
    let applied = apply(file, request, body);
    let test_dimensions = std::mem::take(&mut file.dimensions);
    let test_findings = std::mem::take(&mut file.findings);
    file.dimensions = dimensions;
    file.file_dimensions = file_dimensions;
    file.findings = findings;
    file.context_limitations = limitations;
    file.role_assessment = role;
    file.syntax_checked = syntax;
    file.model = model;
    applied?;
    for (key, dimension) in test_dimensions {
        file.dimensions.insert(format!("test_{key}"), dimension);
    }
    for mut finding in test_findings {
        finding.message = format!("Test portion: {}", finding.message);
        file.findings.push(finding);
    }
    response::update_status(file);
    Ok(())
}

const MIN_SHARED_SPAN: usize = 120;
const EXTRACTION_LIMIT: usize = 8;
/// Named extract tasks need a concentrated follow-up, not only a plurality.
const EXTRACT_CONFIDENCE: f64 = 0.70;

fn concern_of(probabilities: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    let total: f64 = probabilities.values().filter_map(Value::as_f64).sum();
    if total <= 0.0 {
        return None;
    }
    let mass = keys
        .iter()
        .map(|key| {
            probabilities
                .get(*key)
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        })
        .sum::<f64>();
    Some(mass / total)
}

fn overview_basis(basis: &str) -> &str {
    basis
        .split("\nFunction probe:")
        .next()
        .unwrap_or(basis)
        .split("\nShort repeated span:")
        .next()
        .unwrap_or(basis)
}

fn clip_text(text: &str) -> String {
    let trimmed = text.trim();
    let mut out = String::new();
    for (index, ch) in trimmed.chars().enumerate() {
        if index == 160 {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn fragment_span(evidence: &Value) -> usize {
    if let Some(source) = evidence["source"].as_str() {
        let length = source.trim().len();
        if length > 0 {
            return length;
        }
    }
    evidence["locations"]
        .as_array()
        .and_then(|locations| {
            locations
                .iter()
                .filter_map(|location| {
                    let start = location["start_byte"].as_u64()?;
                    let end = location["end_byte"].as_u64()?;
                    Some(end.saturating_sub(start) as usize)
                })
                .max()
        })
        .unwrap_or(0)
}

fn location_label(path: &str, start_line: u64, end_line: u64) -> String {
    format!("{path}:{start_line}–{end_line}")
}

fn pair_detail(names: &str, selected: &[&Value], request: &Value) -> String {
    let file_source = request["state"]["file"]["source"].as_str().unwrap_or("");
    let file_path = request["state"]["file"]["path"].as_str().unwrap_or("");
    let mut primary = String::new();
    let mut copies = Vec::new();
    for (index, operation) in selected.iter().enumerate() {
        let path = operation["path"].as_str().unwrap_or("");
        let start = operation["range"]["start_line"].as_u64().unwrap_or(1) as usize;
        let end = operation["range"]["end_line"]
            .as_u64()
            .unwrap_or(start as u64) as usize;
        let owned;
        let source = if path == file_path {
            file_source
        } else {
            owned = request["state"]["context"]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item["path"].as_str() == Some(path))
                        .and_then(|item| item["source"].as_str())
                })
                .unwrap_or("")
                .to_string();
            owned.as_str()
        };
        let text = line_window(source, start, end);
        if index == 0 {
            primary = text.clone();
        }
        copies.push(json!({"path": path, "start_line": start, "source": text}));
    }
    shared_detail(names, &json!({"source": primary, "surroundings": copies}))
}

fn shared_detail(names: &str, evidence: &Value) -> String {
    let mut detail = format!(
        "Candidate shared implementation at {names}.\nRepeated:\n{}",
        clip_text(evidence["source"].as_str().unwrap_or(""))
    );
    if let Some(copies) = evidence["surroundings"].as_array() {
        for copy in copies.iter().take(2) {
            detail.push_str(&format!(
                "\n{}:{}:\n{}",
                copy["path"].as_str().unwrap_or(""),
                copy["start_line"].as_u64().unwrap_or(1),
                clip_text(copy["source"].as_str().unwrap_or(""))
            ));
        }
    }
    detail
}

fn operation_excerpt(source: &str, operation: &Value) -> String {
    let start = operation["range"]["start_line"].as_u64().unwrap_or(1) as usize;
    let end = operation["range"]["end_line"]
        .as_u64()
        .unwrap_or(start as u64) as usize;
    let text = source
        .lines()
        .skip(start.saturating_sub(1))
        .take(end.saturating_sub(start).saturating_add(1))
        .collect::<Vec<_>>()
        .join("\n");
    clip_text(&text)
}

fn hot_operation(assessment: &Assessment) -> Option<(String, Value, f64)> {
    assessment
        .operations
        .iter()
        .filter_map(|(key, operation)| {
            let probabilities = operation["answer"]["probabilities"].as_object()?;
            let concern = concern_of(probabilities, &["review"])?;
            response::probability_at_least(concern, response::REVIEW_PROBABILITY).then_some((
                key.clone(),
                operation.clone(),
                concern,
            ))
        })
        .max_by(|left, right| left.2.total_cmp(&right.2))
}

fn extraction_choice(operation: &Value) -> Option<(&str, f64, f64)> {
    let extraction = operation.get("extraction")?;
    let probabilities = extraction["probabilities"].as_object()?;
    let total: f64 = probabilities.values().filter_map(Value::as_f64).sum();
    if total <= 0.0 {
        return None;
    }
    let confidence = extraction["confidence"].as_f64().unwrap_or(0.0);
    if let Some(choice) = extraction["choice"].as_str() {
        let mass = probabilities
            .get(choice)
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        return Some((choice, mass / total, confidence));
    }
    let (choice, mass) = probabilities
        .iter()
        .filter_map(|(key, value)| value.as_f64().map(|mass| (key.as_str(), mass)))
        .max_by(|left, right| left.1.total_cmp(&right.1))?;
    Some((choice, mass / total, confidence))
}

fn apply_operations(file: &mut FileResult) -> Result<()> {
    let Some(dimension) = file.dimensions.get_mut("function_simplification") else {
        return Ok(());
    };
    let file_status = dimension.status.clone();
    let Some(assessment) = dimension.refactoring_assessment.as_mut() else {
        return Ok(());
    };
    let Some((key, operation, concern)) = hot_operation(assessment) else {
        return Ok(());
    };
    let evidence = &operation["evidence"];
    anyhow::ensure!(
        evidence["path"] == json!(file.path),
        "Operation does not belong to selected file"
    );
    let name = evidence["name"].as_str().unwrap_or("operation");
    let line = evidence["range"]["start_line"].as_u64().unwrap_or(1);
    assessment.selected_operation = Some(key);
    assessment.selected_locations = vec![evidence.clone()];
    let task = extraction_choice(&operation);
    // A decisive `none` means there is no extract task: the length is the work
    // itself, so the dimension is clear. A plurality `none` is still uncertain
    // and leaves the first-pass judgment unchanged. Named tasks need a
    // concentrated share and confidence before they count as a split.
    let none_task = matches!(
        task,
        Some(("none", share, _)) if response::probability_at_least(share, response::REVIEW_PROBABILITY)
    );
    let note = match task {
        Some(("none", share, _)) if none_task => format!(
            "Function probe: {name} at line {line} scored {concern:.2}. The follow-up is none ({share:.2}); the length is the work itself and this dimension is clear."
        ),
        Some(("none", share, confidence)) => format!(
            "Function probe: {name} at line {line} scored {concern:.2}. The follow-up favored none ({share:.2}, confidence {confidence:.2}) without a decisive share; no extract task was named and the first-pass judgment is unchanged."
        ),
        Some((task, share, confidence)) => format!(
            "Function probe: {name} at line {line} scored {concern:.2}. Follow-up: {task} ({share:.2}, confidence {confidence:.2})."
        ),
        None => format!(
            "Function probe: {name} at line {line} scored {concern:.2}. That score is a place to inspect and does not by itself make the file a review."
        ),
    };
    dimension.decision_basis = format!("{}\n{note}", overview_basis(&dimension.decision_basis));
    if none_task {
        dimension.status = Status::Clear;
        dimension.concern_basis = "extraction-none".into();
    } else {
        dimension.concern_basis = "file-wide-outcome".into();
    }
    file.findings.retain(|finding| finding.rule != IDS[1]);
    let separable = task.is_some_and(|(choice, share, confidence)| {
        matches!(choice, "validation" | "parsing" | "delivery")
            && response::probability_at_least(share, response::REVIEW_PROBABILITY)
            && response::probability_at_least(confidence, EXTRACT_CONFIDENCE)
    });
    if file_status == Status::Review && separable {
        let (task, share, _) = task.unwrap();
        file.findings.push(Finding {
            rule: IDS[1].into(),
            line: line as usize,
            message: format!(
                "File-wide review. {name} at line {line} has a {task} task to extract (follow-up {share:.2}, function score {concern:.2})."
            ),
            action: format!("Consider extracting the {task} task"),
            symbol: Some(name.into()),
            rule_version: crate::catalog::rule_version("function_simplification").into(),
            concern_probability: dimension.concern_probability,
            evidence_complete: dimension.missing_context < response::MISSING_CONTEXT,
        });
    }
    response::update_status(file);
    Ok(())
}

fn apply_fragments(file: &mut FileResult) -> Result<()> {
    let Some(dimension) = file.dimensions.get_mut("shared_logic") else {
        return Ok(());
    };
    let Some(assessment) = dimension.refactoring_assessment.as_mut() else {
        return Ok(());
    };
    let best = assessment
        .fragments
        .iter()
        .filter_map(|(key, fragment)| {
            let probabilities = fragment["answer"]["probabilities"].as_object()?;
            let total: f64 = probabilities.values().filter_map(Value::as_f64).sum();
            let concern: f64 = ["shared_setup", "shared_construction", "shared_algorithm"]
                .iter()
                .map(|key| {
                    probabilities
                        .get(*key)
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0)
                })
                .sum();
            (total > 0.0).then_some((key.clone(), fragment.clone(), concern / total))
        })
        .max_by(|a, b| a.2.total_cmp(&b.2));
    let Some((key, fragment, concern)) =
        best.filter(|(_, _, p)| response::probability_at_least(*p, response::REVIEW_PROBABILITY))
    else {
        return Ok(());
    };
    let span = fragment_span(&fragment["evidence"]);
    if span < MIN_SHARED_SPAN {
        dimension.decision_basis = format!(
            "{}\nShort repeated span: {span} bytes stayed a probability ({concern:.2}) and was not printed as a finding.",
            overview_basis(&dimension.decision_basis)
        );
        return Ok(());
    }
    let locations = fragment["evidence"]["locations"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing fragment source locations"))?;
    let primary = locations
        .iter()
        .find(|location| location["path"] == json!(file.path))
        .ok_or_else(|| anyhow::anyhow!("Fragment does not involve selected file"))?;
    let names = locations
        .iter()
        .map(|location| {
            location_label(
                location["path"].as_str().unwrap_or(""),
                location["start_line"].as_u64().unwrap_or(1),
                location["end_line"].as_u64().unwrap_or(1),
            )
        })
        .collect::<Vec<_>>()
        .join(" and ");
    let detail = shared_detail(&names, &fragment["evidence"]);
    assessment.selected_fragment = Some(key);
    assessment.selected_locations = locations.iter().map(|location| json!({"path":location["path"],"range":{"start_line":location["start_line"],"end_line":location["end_line"]},"name":"repeated source fragment"})).collect();
    dimension.probabilities = serde_json::from_value(fragment["answer"]["probabilities"].clone())?;
    let total: f64 = dimension.probabilities.values().sum();
    dimension.concern_probability = concern;
    dimension.confidence = fragment["answer"]["confidence"].as_f64().unwrap_or(0.0);
    dimension.missing_context = dimension
        .probabilities
        .get("context")
        .copied()
        .unwrap_or(0.0)
        / total;
    dimension.evidence_sufficiency = Some(1.0 - dimension.missing_context);
    dimension.concern_basis = "direct-shared-fragment-responsibility".into();
    dimension.decision_basis = detail.clone();
    dimension.status = Status::Review;
    file.findings.retain(|finding| finding.rule != IDS[2]);
    file.findings.push(Finding {
        rule: IDS[2].into(),
        line: primary["start_line"].as_u64().unwrap_or(1) as usize,
        message: detail,
        action: "Consider sharing the repeated implementation".into(),
        symbol: Some(names),
        rule_version: crate::catalog::rule_version("shared_logic").into(),
        concern_probability: concern,
        evidence_complete: dimension.missing_context < response::MISSING_CONTEXT,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{Project, args, run};
    struct Judge {
        calls: usize,
        weights: Vec<(&'static str, f64)>,
    }
    impl crate::transport::Evaluator for Judge {
        fn evaluate(&mut self, request: &Value) -> Result<Value> {
            self.calls += 1;
            if request["state"].get("focused").is_none() {
                assert!(request["state"]["maintainability_version"].is_number());
            }
            let answers = request["questions"].as_object().unwrap().iter().map(|(key,q)| {
                if q["type"] == "noul" {
                    return (key.clone(), json!({"type":"noul","noul":0.02}));
                }
                let location = key.ends_with("_location");
                let candidate_mass: f64 = self.weights.iter().filter(|(k,_)| k.starts_with('p') || k.starts_with('o')).map(|(_,p)|p).sum();
                let weights = q["criteria"].as_object().unwrap().keys().map(|k| {
                    let p = if location {
                        if k == "none" { 1.0-candidate_mass }
                        else { self.weights.iter().find(|(n,_)|n==k).map_or(0.0,|(_,p)|*p) }
                    } else if k == "review" { candidate_mass + self.weights.iter().find(|(n,_)|*n=="review").map_or(0.0,|(_,p)|*p) }
                    else { self.weights.iter().find(|(n,_)|n==k).map_or(0.0,|(_,p)|*p) };
                    (k.clone(),json!(p))
                }).collect::<serde_json::Map<_,_>>();
                let choice = weights.iter().max_by(|a,b|a.1.as_f64().unwrap().total_cmp(&b.1.as_f64().unwrap())).unwrap().0;
                (key.clone(),json!({"type":"choice","choice":choice,"confidence":0.8,"probabilities":weights}))
            }).collect::<serde_json::Map<_,_>>();
            Ok(
                json!({"model":request["model"],"answers":answers,"usage":{"input_tokens":10,"output_tokens":10}}),
            )
        }
    }
    const SOURCE: &str = "def save_order(order):\n    return database.write(order)\n\ndef render_banner(user):\n    return '<h1>' + user.name + '</h1>'\n";
    #[test]
    fn operation_judgments_localize_supported_reviews_without_clearing_overview() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        let mut options = args();
        options.rules = vec!["function_simplification".into()];
        let report = run(
            &p,
            &options,
            &mut Judge {
                calls: 0,
                weights: vec![("clear", 1.0)],
            },
        );
        for review in [0.79, 0.8] {
            let mut file = report.files[0].clone();
            let assessment = file
                .dimensions
                .get_mut("function_simplification")
                .unwrap()
                .refactoring_assessment
                .as_mut()
                .unwrap();
            assessment.operations.clear();
            assessment.operations.insert("operation_probe_1".into(),json!({"answer":{"confidence":0.8,"probabilities":{"review":review,"clear":1.0-review,"context":0.0}},"evidence":{"path":"app.py","name":"render_banner","range":{"start_line":4,"end_line":5}}}));
            apply_operations(&mut file).unwrap();
            let dimension = &file.dimensions["function_simplification"];
            assert_eq!(dimension.status, Status::Clear);
            assert_eq!(
                dimension.refactoring_assessment.as_ref().unwrap().outcome["probabilities"]["clear"],
                1.0
            );
            assert!(file.findings.is_empty());
            if review >= 0.8 {
                assert!(dimension.decision_basis.contains("render_banner"));
                assert_eq!(
                    dimension
                        .refactoring_assessment
                        .as_ref()
                        .unwrap()
                        .selected_operation
                        .as_deref(),
                    Some("operation_probe_1")
                );
            }
        }
        let mut file = report.files[0].clone();
        file.dimensions
            .get_mut("function_simplification")
            .unwrap()
            .status = Status::Uncertain;
        apply_operations(&mut file).unwrap();
        assert_eq!(
            file.dimensions["function_simplification"].status,
            Status::Uncertain
        );
        file.dimensions
            .get_mut("function_simplification")
            .unwrap()
            .status = Status::Review;
        apply_operations(&mut file).unwrap();
        assert_eq!(
            file.dimensions["function_simplification"].status,
            Status::Review
        );
    }

    #[test]
    fn fragment_classification_keeps_overview_and_requires_supported_concern() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        let mut options = args();
        options.rules = vec!["shared_logic".into()];
        let report = run(
            &p,
            &options,
            &mut Judge {
                calls: 0,
                weights: vec![("clear", 1.0)],
            },
        );
        let repeated = "normalized = value.strip().lower()\nrecord = dict(name=normalized, enabled=True, ready=True)\nreturn database.save(record)\n";
        assert!(repeated.len() >= 120);
        for (setup, construction, source, expected) in [
            (0.59, 0.2, repeated, Status::Clear),
            (0.6, 0.2, "record construction", Status::Clear),
            (0.6, 0.2, repeated, Status::Review),
        ] {
            let mut file = report.files[0].clone();
            file.dimensions.get_mut("shared_logic").unwrap().refactoring_assessment.as_mut().unwrap().fragments.insert("shared_logic_fragment_0".into(),json!({"answer":{"confidence":0.8,"probabilities":{"shared_setup":setup,"shared_construction":construction,"idiom":1.0-setup-construction}},"evidence":{"source":source,"surroundings":[{"path":"app.py","start_line":1,"source":"def save_order(order):\n    return database.write(order)"},{"path":"app.py","start_line":4,"source":"def render_banner(user):\n    return user.name"}],"locations":[{"path":"app.py","start_line":1,"end_line":2},{"path":"app.py","start_line":4,"end_line":5}]}}));
            apply_fragments(&mut file).unwrap();
            let dimension = &file.dimensions["shared_logic"];
            assert_eq!(dimension.status, expected);
            assert_eq!(
                dimension.refactoring_assessment.as_ref().unwrap().outcome["probabilities"]["clear"],
                1.0
            );
            if expected == Status::Review {
                assert_eq!(file.findings.len(), 1);
                assert_eq!(file.findings[0].line, 1);
                assert!(file.findings[0].message.contains("Repeated:"));
                assert!(file.findings[0].message.contains("save_order"));
                assert!(file.findings[0].message.contains("render_banner"));
                assert_eq!(
                    dimension.concern_basis,
                    "direct-shared-fragment-responsibility"
                );
                assert_eq!(
                    dimension
                        .refactoring_assessment
                        .as_ref()
                        .unwrap()
                        .selected_locations
                        .len(),
                    2
                );
            } else {
                assert!(file.findings.is_empty());
                if source == "record construction" {
                    assert!(dimension.decision_basis.contains("Short repeated span"));
                }
            }
        }
    }
    #[test]
    fn extraction_follow_up_names_a_task_only_when_the_file_is_already_review() {
        let project = Project::new();
        project.write("app.py", SOURCE);
        let options = args();
        let report = run(
            &project,
            &options,
            &mut Judge {
                calls: 0,
                weights: vec![("clear", 1.0)],
            },
        );
        let mut file = report.files[0].clone();
        let evidence =
            json!({"path":"app.py","name":"render_banner","range":{"start_line":4,"end_line":5}});
        file.dimensions.get_mut("function_simplification").unwrap().refactoring_assessment.as_mut().unwrap().operations.insert("operation_probe_1".into(), json!({"answer":{"confidence":0.8,"probabilities":{"review":0.9,"clear":0.1,"context":0.0}},"evidence":evidence}));
        let requests = extraction_requests(
            &crate::inventory::collect(&options, &project.context(), &[])
                .unwrap()
                .remove(0),
            &file,
            &options,
        )
        .unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]["questions"]["function_simplification_extract_0"]["criteria"]["none"]
                .is_object()
        );
        file.dimensions
            .get_mut("function_simplification")
            .unwrap()
            .status = Status::Review;
        let answer = json!({"type":"choice","choice":"parsing","confidence":0.8,"probabilities":{"parsing":0.86,"validation":0.04,"delivery":0.05,"none":0.05}});
        apply_extraction(
            &mut file,
            &requests[0],
            &json!({"answers":{"function_simplification_extract_0":answer}}),
        )
        .unwrap();
        assert_eq!(
            file.dimensions["function_simplification"].status,
            Status::Review
        );
        assert_eq!(file.findings.len(), 1);
        assert!(file.findings[0].message.contains("parsing"));
        assert_eq!(file.findings[0].line, 4);
        let none = json!({"type":"choice","choice":"none","confidence":0.8,"probabilities":{"parsing":0.05,"validation":0.04,"delivery":0.05,"none":0.86}});
        file.findings.clear();
        file.dimensions
            .get_mut("function_simplification")
            .unwrap()
            .status = Status::Review;
        apply_extraction(
            &mut file,
            &requests[0],
            &json!({"answers":{"function_simplification_extract_0":none}}),
        )
        .unwrap();
        assert_eq!(
            file.dimensions["function_simplification"].status,
            Status::Clear
        );
        assert_eq!(
            file.dimensions["function_simplification"].concern_basis,
            "extraction-none"
        );
        assert!(file.findings.is_empty());
        assert!(
            file.dimensions["function_simplification"]
                .decision_basis
                .contains("this dimension is clear")
        );
        let original_basis = file.file_dimensions["function_simplification"]
            .decision_basis
            .clone();
        assert!(!original_basis.contains("Function probe:"));
        assert_eq!(
            file.file_dimensions["function_simplification"].concern_basis,
            "direct-maintainability-outcome"
        );
        let weak = json!({"type":"choice","choice":"none","confidence":0.4,"probabilities":{"parsing":0.2,"validation":0.1,"delivery":0.14,"none":0.56}});
        file.dimensions
            .get_mut("function_simplification")
            .unwrap()
            .status = Status::Review;
        apply_extraction(
            &mut file,
            &requests[0],
            &json!({"answers":{"function_simplification_extract_0":weak}}),
        )
        .unwrap();
        assert_eq!(
            file.dimensions["function_simplification"].status,
            Status::Review,
            "a plurality none is not decisive and must not clear the dimension"
        );
        assert!(file.findings.is_empty());
        assert!(
            file.dimensions["function_simplification"]
                .decision_basis
                .contains("without a decisive share")
        );
        assert_eq!(
            file.file_dimensions["function_simplification"].decision_basis,
            original_basis
        );
        let low_confidence = json!({"type":"choice","choice":"parsing","confidence":0.4,"probabilities":{"parsing":0.86,"validation":0.04,"delivery":0.05,"none":0.05}});
        apply_extraction(
            &mut file,
            &requests[0],
            &json!({"answers":{"function_simplification_extract_0":low_confidence}}),
        )
        .unwrap();
        assert!(file.findings.is_empty());
        file.findings.clear();
        file.dimensions
            .get_mut("function_simplification")
            .unwrap()
            .status = Status::Clear;
        apply_extraction(
            &mut file,
            &requests[0],
            &json!({"answers":{"function_simplification_extract_0":none}}),
        )
        .unwrap();
        assert_eq!(
            file.dimensions["function_simplification"].status,
            Status::Clear
        );
        assert!(file.findings.is_empty());
        assert!(
            file.dimensions["function_simplification"]
                .decision_basis
                .contains("none")
        );
    }

    #[test]
    fn focused_recheck_skips_a_decisive_file_wide_verdict() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        let mut options = args();
        options.rules = vec![KEYS[2].into()];
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let request = request(&inputs[0], &options).unwrap();
        use crate::transport::Evaluator;
        let mut judge = Judge {
            calls: 0,
            weights: vec![("clear", 1.0)],
        };
        let mut body = judge.evaluate(&request).unwrap();
        body["answers"]["shared_logic"] = json!({"type":"choice","choice":"review","confidence":1.0,"probabilities":{"clear":0.0,"context":0.0,"not_applicable":0.0,"review":1.0}});
        body["answers"]["shared_logic_location"] = json!({"type":"choice","choice":"none","confidence":0.4,"probabilities":{"none":0.51,"p0":0.49}});
        crate::response::validate(&body, &request).unwrap();
        let mut file = inputs[0].result.clone();
        apply(&mut file, &request, &body).unwrap();
        assert_eq!(file.dimensions[KEYS[2]].status, Status::Uncertain);
        assert!(file.dimensions[KEYS[2]].probabilities["review"] >= 0.8);
        let followups = focused_requests(&inputs[0], &file, &options).unwrap();
        assert!(followups.is_empty());
        file.dimensions
            .get_mut("shared_logic")
            .unwrap()
            .probabilities
            .insert("review".into(), 0.5);
        file.dimensions
            .get_mut("shared_logic")
            .unwrap()
            .probabilities
            .insert("clear".into(), 0.4);
        let undecided = focused_requests(&inputs[0], &file, &options).unwrap();
        assert_eq!(undecided.len(), 1);
    }

    #[test]
    fn fragment_evidence_is_shared_bounded_and_independent_of_selected_rules() {
        let p = Project::new();
        let source = (0..24).map(|i| format!("def operation_{i}(value):\n    normalized = value.strip().lower()\n    record = dict(name=normalized, enabled=True)\n    return save(record)\n\n")).collect::<String>();
        p.write("app.py", &source);
        let mut options = args();
        p.context().configure(&mut options).unwrap();
        let input = crate::inventory::collect(&options, &p.context(), &[])
            .unwrap()
            .remove(0);
        let full = request(&input, &options).unwrap();
        let fragments = full["state"]["repeated_fragments"].as_array().unwrap();
        assert!(!fragments.is_empty());
        let bytes: usize = fragments
            .iter()
            .flat_map(|f| f["surroundings"].as_array().unwrap())
            .map(|e| serde_json::to_vec(e).unwrap().len())
            .sum();
        assert!(bytes > 0 && bytes <= 4096);
        assert!(
            full["state"]["limitations"]["fragment_excerpts_omitted"]
                .as_u64()
                .unwrap()
                > 0
        );
        assert_eq!(full["state"]["file"]["source"], source);
        for (key, question) in full["questions"].as_object().unwrap() {
            if key.starts_with("shared_logic_fragment_") {
                let index = question["instructions"]["fragment_index"].as_u64().unwrap() as usize;
                assert!(index < fragments.len());
                assert!(question["instructions"].get("fragment").is_none());
            }
        }
        options.rules = vec!["file_organization".into()];
        let selected = request(&input, &options).unwrap();
        for key in [
            "file",
            "context",
            "operations",
            "pairs",
            "repeated_fragments",
            "limitations",
        ] {
            assert_eq!(selected["state"][key], full["state"][key]);
        }
    }

    #[test]
    fn intentional_fragment_probability_does_not_establish_shared_implementation() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        let options = args();
        let mut report = run(
            &p,
            &options,
            &mut Judge {
                calls: 0,
                weights: vec![("clear", 1.0)],
            },
        );
        let file = &mut report.files[0];
        let probabilities = json!({"intentional_sequence":0.9,"shared_setup":0.1,"context":0.0});
        file.dimensions.get_mut("shared_logic").unwrap().refactoring_assessment.as_mut().unwrap().fragments.insert("shared_logic_fragment_0".into(),json!({"answer":{"confidence":0.8,"probabilities":probabilities},"evidence":{"locations":[]}}));
        apply_fragments(file).unwrap();
        assert_eq!(file.dimensions["shared_logic"].status, Status::Clear);
        assert!(file.findings.is_empty());
        assert_eq!(
            file.dimensions["shared_logic"]
                .refactoring_assessment
                .as_ref()
                .unwrap()
                .fragments["shared_logic_fragment_0"]["answer"]["probabilities"],
            probabilities
        );
    }

    #[test]
    fn defaults_batch_only_new_rules_and_cache_tracks_source_and_explicit_context() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        p.write("peer.py", "def helper():\n    return 1\n");
        let mut options = args();
        options.paths = vec!["app.py".into()];
        p.context().configure(&mut options).unwrap();
        let mut judge = Judge {
            calls: 0,
            weights: vec![("clear", 1.0)],
        };
        let first = run(&p, &options, &mut judge);
        assert_eq!(first.api_requests, 1);
        assert_eq!(first.decision_policy["clear_probability"], 0.8);
        assert_eq!(first.decision_policy["location_probability"], 0.65);
        assert_eq!(first.decision_policy["extract_confidence"], 0.70);
        assert_eq!(first.files[0].dimensions.len(), 3);
        assert!(first.files[0].context_files.is_empty());
        assert_eq!(first.files[0].status, Status::Clear);
        assert_eq!(run(&p, &options, &mut judge).api_requests, 0);
        options.rules = vec![KEYS[0].into()];
        assert_eq!(run(&p, &options, &mut judge).api_requests, 0);
        options.context = vec!["peer.py".into()];
        assert_eq!(run(&p, &options, &mut judge).api_requests, 1);
        p.write("peer.py", "def helper():\n    return 2\n");
        options.cache_only = true;
        let stale = run(&p, &options, &mut judge);
        assert_eq!(stale.api_requests, 0);
        assert_eq!(stale.files[0].status, Status::Error);
    }
    #[test]
    fn semantic_split_can_select_independent_functions_without_shared_state() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        let mut options = args();
        options.rules = vec![KEYS[0].into()];
        p.context().configure(&mut options).unwrap();
        let mut judge = Judge {
            calls: 0,
            weights: vec![("p0", 0.85), ("clear", 0.15)],
        };
        let report = run(&p, &options, &mut judge);
        let f = &report.files[0];
        assert_eq!(f.status, Status::Review);
        assert_eq!(f.findings[0].line, 1);
        assert!(f.findings[0].message.contains("render_banner"));
        assert!(f.findings[0].message.contains("save_order"));
        assert_eq!(f.dimensions[KEYS[0]].probabilities["review"], 0.85);
        assert_eq!(
            f.dimensions[KEYS[0]]
                .refactoring_assessment
                .as_ref()
                .unwrap()
                .location["probabilities"]["p0"],
            0.85
        );
    }
    #[test]
    fn location_selection_is_not_a_verdict_and_none_does_not_invent_a_boundary() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        let mut options = args();
        options.rules = vec![KEYS[0].into()];
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let request = request(&inputs[0], &options).unwrap();
        use crate::transport::Evaluator;
        let mut judge = Judge {
            calls: 0,
            weights: vec![("clear", 1.0)],
        };
        let mut body = judge.evaluate(&request).unwrap();
        body["answers"]["file_organization_location"] = json!({"type":"choice","choice":"p0","confidence":1.0,"probabilities":{"none":0.0,"p0":1.0}});
        crate::response::validate(&body, &request).unwrap();
        let mut file = inputs[0].result.clone();
        apply(&mut file, &request, &body).unwrap();
        assert_eq!(file.status, Status::Clear);
        assert!(file.findings.is_empty());
        body["answers"]["file_organization"] = json!({"type":"choice","choice":"review","confidence":1.0,"probabilities":{"clear":0.0,"context":0.0,"not_applicable":0.0,"review":1.0}});
        body["answers"]["file_organization_location"] = json!({"type":"choice","choice":"none","confidence":0.8,"probabilities":{"none":0.9,"p0":0.1}});
        crate::response::validate(&body, &request).unwrap();
        apply(&mut file, &request, &body).unwrap();
        assert_eq!(file.status, Status::Review);
        assert_eq!(file.findings.len(), 1);
        assert!(file.findings[0].symbol.is_none());
        assert!(!file.findings[0].evidence_complete);
        assert!(
            file.dimensions[KEYS[0]]
                .refactoring_assessment
                .as_ref()
                .unwrap()
                .selected_locations
                .is_empty()
        );
        body["answers"]["file_organization_location"] = json!({"type":"choice","choice":"p0","confidence":0.1,"probabilities":{"none":0.49,"p0":0.51}});
        apply(&mut file, &request, &body).unwrap();
        assert_eq!(file.status, Status::Review);
        assert_eq!(file.findings.len(), 1);
        assert!(file.findings[0].symbol.is_none());
        assert!(!file.findings[0].evidence_complete);
        assert!(
            file.findings[0]
                .message
                .contains("no supplied candidate localizes its boundary")
        );
        assert!(
            file.dimensions[KEYS[0]]
                .refactoring_assessment
                .as_ref()
                .unwrap()
                .selected_locations
                .is_empty()
        );
        body["answers"]["file_organization_location"] = json!({"type":"choice","choice":"p0","confidence":0.8,"probabilities":{"none":0.1,"p0":0.9}});
        apply(&mut file, &request, &body).unwrap();
        assert_eq!(file.status, Status::Review);
        assert_eq!(file.findings.len(), 1);
        assert!(file.findings[0].evidence_complete);
        assert!(file.findings[0].symbol.is_some());
        body["answers"]["file_organization"] = json!({"type":"choice","choice":"review","confidence":0.1,"probabilities":{"clear":0.4,"context":0.0,"not_applicable":0.0,"review":0.6}});
        apply(&mut file, &request, &body).unwrap();
        assert_eq!(file.status, Status::Uncertain);
        assert!(file.findings.is_empty());
        assert!(
            file.dimensions[KEYS[0]]
                .decision_basis
                .contains("does not establish a need to refactor")
        );
    }

    #[test]
    fn unlocalized_shared_logic_awaits_a_pair_follow_up() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        let mut options = args();
        options.rules = vec![KEYS[2].into()];
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let request = request(&inputs[0], &options).unwrap();
        use crate::transport::Evaluator;
        let mut judge = Judge {
            calls: 0,
            weights: vec![("clear", 1.0)],
        };
        let mut body = judge.evaluate(&request).unwrap();
        body["answers"]["shared_logic"] = json!({"type":"choice","choice":"review","confidence":1.0,"probabilities":{"clear":0.0,"context":0.0,"not_applicable":0.0,"review":1.0}});
        body["answers"]["shared_logic_location"] = json!({"type":"choice","choice":"none","confidence":0.8,"probabilities":{"none":0.9,"p0":0.1}});
        crate::response::validate(&body, &request).unwrap();
        let mut file = inputs[0].result.clone();
        apply(&mut file, &request, &body).unwrap();
        assert_eq!(file.dimensions[KEYS[2]].status, Status::Uncertain);
        assert!(file.findings.is_empty());
        assert!(
            file.dimensions[KEYS[2]]
                .decision_basis
                .contains("focused follow-up")
        );
    }

    #[test]
    fn focused_shared_logic_review_quotes_both_copies() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        let mut options = args();
        options.rules = vec![KEYS[2].into()];
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let request = request(&inputs[0], &options).unwrap();
        use crate::transport::Evaluator;
        let mut judge = Judge {
            calls: 0,
            weights: vec![("clear", 1.0)],
        };
        let mut body = judge.evaluate(&request).unwrap();
        body["answers"]["shared_logic"] = json!({"type":"choice","choice":"review","confidence":0.5,"probabilities":{"clear":0.4,"context":0.0,"not_applicable":0.0,"review":0.6}});
        crate::response::validate(&body, &request).unwrap();
        let mut file = inputs[0].result.clone();
        apply(&mut file, &request, &body).unwrap();
        assert_eq!(file.dimensions[KEYS[2]].status, Status::Uncertain);
        let fragment = json!({
            "source": "normalized = value.strip().lower()\nrecord = dict(name=normalized, enabled=True, ready=True)\nreturn database.save(record)\n",
            "surroundings": [
                {"path":"app.py","start_line":1,"source":"def save_order(order):\n    return database.write(order)"},
                {"path":"app.py","start_line":4,"source":"def render_banner(user):\n    return user.name"}
            ],
            "locations": [
                {"path":"app.py","start_line":1,"end_line":2},
                {"path":"app.py","start_line":4,"end_line":5}
            ]
        });
        let focused = json!({
            "model": options.model,
            "state": {
                "focused": "shared_logic",
                "file": {"path":"app.py","source":"def save_order(order):\n    return database.write(order)"},
                "focus_evidence": [fragment]
            },
            "questions": { "shared_logic": crate::questions::recheck(2, "") }
        });
        let focused_body = json!({"answers":{"shared_logic":{"type":"choice","choice":"review","confidence":0.9,"probabilities":{"clear":0.05,"review":0.95}}}});
        apply_focused(&mut file, &focused, &focused_body).unwrap();
        assert_eq!(file.dimensions[KEYS[2]].status, Status::Review);
        assert_eq!(file.findings.len(), 1);
        assert!(file.findings[0].message.contains("Repeated:"));
        assert!(file.findings[0].message.contains("save_order"));
        assert!(file.findings[0].message.contains("render_banner"));
        let short = json!({
            "source": "previous = super::evaluate::previous_judgments(Some(&report), false);",
            "locations": [
                {"path":"app.py","start_line":1,"end_line":1},
                {"path":"app.py","start_line":4,"end_line":4}
            ]
        });
        let short_focus = json!({
            "model": options.model,
            "state": {
                "focused": "shared_logic",
                "file": {"path":"app.py","source":"def save_order(order):\n    return database.write(order)"},
                "focus_evidence": [short]
            },
            "questions": { "shared_logic": crate::questions::recheck(2, "") }
        });
        file.findings.clear();
        file.dimensions.get_mut("shared_logic").unwrap().status = Status::Uncertain;
        apply_focused(&mut file, &short_focus, &focused_body).unwrap();
        assert!(file.findings.is_empty());
        assert!(
            file.dimensions["shared_logic"]
                .decision_basis
                .contains("Short repeated span")
        );
    }

    #[test]
    fn focused_pair_follow_up_resolves_a_weak_first_pass_location() {
        let p = Project::new();
        p.write("app.py", SOURCE);
        let mut options = args();
        options.rules = vec![KEYS[2].into()];
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let request = request(&inputs[0], &options).unwrap();
        use crate::transport::Evaluator;
        let mut judge = Judge {
            calls: 0,
            weights: vec![("clear", 1.0)],
        };
        let mut body = judge.evaluate(&request).unwrap();
        body["answers"]["shared_logic"] = json!({"type":"choice","choice":"review","confidence":1.0,"probabilities":{"clear":0.0,"context":0.0,"not_applicable":0.0,"review":1.0}});
        let location_keys: Vec<String> = request["questions"]["shared_logic_location"]["criteria"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        let mut location_probs = serde_json::Map::new();
        let rest = 0.49 / (location_keys.len() as f64 - 1.0).max(1.0);
        for key in &location_keys {
            location_probs.insert(key.clone(), json!(if key == "p0" { 0.51 } else { rest }));
        }
        body["answers"]["shared_logic_location"] =
            json!({"type":"choice","choice":"p0","confidence":0.4,"probabilities":location_probs});
        crate::response::validate(&body, &request).unwrap();
        let mut file = inputs[0].result.clone();
        apply(&mut file, &request, &body).unwrap();
        assert_eq!(file.dimensions[KEYS[2]].status, Status::Uncertain);
        assert!(file.findings.is_empty());
        assert!(
            file.dimensions[KEYS[2]]
                .decision_basis
                .contains("Review without a supported location")
        );
        let candidate = json!({
            "key": "p0",
            "names": "app.py:1–2 and app.py:4–5",
            "locations": [
                {"path":"app.py","start_line":1,"end_line":2},
                {"path":"app.py","start_line":4,"end_line":5}
            ],
            "evidence": {
                "source": "normalized = value.strip().lower()\nrecord = dict(name=normalized, enabled=True, ready=True)\nreturn database.save(record)\n",
                "surroundings": [
                    {"path":"app.py","start_line":1,"source":"def save_order(order):\n    return database.write(order)"},
                    {"path":"app.py","start_line":4,"source":"def render_banner(user):\n    return user.name"}
                ]
            }
        });
        let focused = json!({
            "model": options.model,
            "state": {
                "focused": "shared_logic",
                "file": {"path":"app.py","source":"def save_order(order):\n    return database.write(order)"},
                "focus_evidence": [candidate["evidence"].clone()],
                "location_candidates": [candidate]
            },
            "questions": {
                "shared_logic": crate::questions::recheck(2, ""),
                "shared_logic_location": {"type":"choice","instructions": crate::questions::location_instructions(2), "criteria": {"none": "No pair.", "p0": "Operations app.py:1–2 and app.py:4–5 repeat the same implementation."}}
            }
        });
        let focused_body = json!({"answers":{
            "shared_logic":{"type":"choice","choice":"review","confidence":0.9,"probabilities":{"clear":0.05,"review":0.95}},
            "shared_logic_location":{"type":"choice","choice":"p0","confidence":0.9,"probabilities":{"none":0.05,"p0":0.95}}
        }});
        apply_focused(&mut file, &focused, &focused_body).unwrap();
        assert_eq!(file.dimensions[KEYS[2]].status, Status::Review);
        assert_eq!(file.findings.len(), 1);
        assert!(file.findings[0].message.contains("Repeated:"));
        assert!(file.findings[0].message.contains("save_order"));
        assert!(file.findings[0].message.contains("render_banner"));
        assert_eq!(
            file.dimensions[KEYS[2]]
                .refactoring_assessment
                .as_ref()
                .unwrap()
                .selected_locations
                .len(),
            2
        );
    }

    #[test]
    fn data_declarations_are_not_function_or_shared_implementation_candidates() {
        let p = Project::new();
        p.write("module.ts", "const labels = { a: 'A', b: 'B' };\nexport function label(key: string) { return labels[key]; }\nexport const exists = (key: string) => key in labels;\n");
        let mut options = args();
        p.context().configure(&mut options).unwrap();
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let r = request(&inputs[0], &options).unwrap();
        assert_eq!(r["state"]["operations"][0]["name"], "labels");
        assert_eq!(r["state"]["operations"][0]["callable"], false);
        assert!(
            r["questions"]["function_simplification_location"]["criteria"]
                .get("o0")
                .is_none()
        );
        assert!(
            r["questions"]["function_simplification_location"]["criteria"]
                .get("o1")
                .is_some()
        );
        assert!(
            r["questions"]["function_simplification_location"]["criteria"]
                .get("o2")
                .is_some()
        );
        assert_eq!(
            r["questions"]["shared_logic_location"]["criteria"]
                .as_object()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn uncertainty_missing_context_and_omitted_candidates_are_not_cleared() {
        for (weights, status) in [
            (vec![("clear", 0.4), ("p0", 0.6)], Status::Uncertain),
            (vec![("context", 0.8), ("p0", 0.2)], Status::NeedsContext),
        ] {
            let p = Project::new();
            p.write("app.py", SOURCE);
            let mut options = args();
            options.rules = vec![KEYS[0].into()];
            let report = run(&p, &options, &mut Judge { calls: 0, weights });
            assert_eq!(report.files[0].status, status);
            assert!(report.files[0].findings.is_empty());
        }
        let p = Project::new();
        p.write(
            "app.py",
            &(0..70)
                .map(|i| format!("def f{i}():\n    return {i}\n"))
                .collect::<String>(),
        );
        let mut options = args();
        p.context().configure(&mut options).unwrap();
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let r = request(&inputs[0], &options).unwrap();
        assert_eq!(r["state"]["limitations"]["operations_omitted"], 6);
        assert!(r["state"]["limitations"]["pairs_omitted"].as_u64().unwrap() > 0);
        assert_eq!(r["state"]["maintainability_version"], 9);
        for (key, q) in r["questions"].as_object().unwrap() {
            assert!(q["criteria"].as_object().unwrap().len() <= 255, "{key}");
            assert!(q["instructions"].get("version").is_none(), "{key}");
            assert!(q["instructions"].get("task").is_none(), "{key}");
            if key.starts_with("role_") {
                for side in ["true", "false"] {
                    assert!(q["criteria"][side]["what"].is_string(), "{key}");
                    assert!(q["criteria"][side]["examples"].is_array(), "{key}");
                }
            } else if key.ends_with("_location") {
                assert!(q["criteria"]["none"].is_string(), "{key}");
            } else {
                assert!(q["criteria"]["context"]["what"].is_string(), "{key}");
                assert!(q["criteria"]["context"]["not_for"].is_string(), "{key}");
                assert!(q["criteria"]["context"]["examples"].is_array(), "{key}");
            }
        }
    }

    #[test]
    fn line_budget_is_evidence_and_never_a_deterministic_verdict() {
        let p = Project::new();
        let tall = (0..60)
            .map(|i| format!("def f{i}():\n    return {i}\n"))
            .collect::<String>();
        p.write("app.py", &tall);
        let mut options = args();
        options.rules = vec![KEYS[0].into()];
        p.context().configure(&mut options).unwrap();
        let inputs = crate::inventory::collect(&options, &p.context(), &[]).unwrap();
        let request = request(&inputs[0], &options).unwrap();
        assert_eq!(request["state"]["file"]["line_count"], 120);
        assert_eq!(
            request["state"]["file"]["line_budget"],
            crate::options::DEFAULT_LINE_BUDGET
        );
        let focus = request["questions"]["file_organization"]["instructions"]["focus"]
            .as_str()
            .unwrap();
        assert!(focus.contains("line_budget"));
        let report = run(
            &p,
            &options,
            &mut Judge {
                calls: 0,
                weights: vec![("clear", 1.0)],
            },
        );
        assert_eq!(
            report.files[0].dimensions["file_organization"].status,
            Status::Clear,
            "a tall clear file stays clear; size is not a verdict"
        );
        let mut options = args();
        options.rules = vec![KEYS[0].into()];
        options.line_budget = 50;
        let report = run(
            &p,
            &options,
            &mut Judge {
                calls: 0,
                weights: vec![("clear", 1.0)],
            },
        );
        assert_eq!(
            report.files[0].dimensions["file_organization"].status,
            Status::Clear,
            "exceeding a tighter budget still does not decide the verdict"
        );
    }

    #[test]
    fn focused_recheck_sends_only_the_undecided_operation() {
        let project = Project::new();
        project.write(
            "app.py",
            "def keep_ready(value):\n    return value\n\ndef load_inputs(value):\n    name = value.strip().lower()\n    return {\"name\": name}\n",
        );
        let mut options = args();
        options.refresh = true;
        struct Focus {
            calls: usize,
            source: String,
            decisive: bool,
        }
        impl crate::transport::Evaluator for Focus {
            fn evaluate(&mut self, request: &Value) -> Result<Value> {
                self.calls += 1;
                let focused =
                    request["state"]["focused"].as_str() == Some("function_simplification");
                if focused {
                    self.source = request["state"]["file"]["source"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                }
                let mut answers = serde_json::Map::new();
                for (name, question) in request["questions"].as_object().unwrap() {
                    if question["type"] == "noul" {
                        answers.insert(name.clone(), json!({"type":"noul","noul":0.02}));
                        continue;
                    }
                    let criteria = question["criteria"].as_object().unwrap();
                    let mut chosen = if criteria.contains_key("clear") {
                        "clear"
                    } else if criteria.contains_key("idiom") {
                        "idiom"
                    } else {
                        criteria.keys().next().unwrap().as_str()
                    };
                    let probe = name.starts_with("operation_probe_")
                        && request["questions"][name]["instructions"]["source"]
                            .as_str()
                            .unwrap_or("")
                            .contains("load_inputs");
                    if !focused && (name == "function_simplification" || probe) {
                        chosen = "review";
                    }
                    if focused && !self.decisive {
                        chosen = "review";
                    }
                    let probabilities = criteria
                        .keys()
                        .map(|key| {
                            let probability = if key == chosen {
                                if !focused && (name == "function_simplification" || probe) {
                                    if name == "function_simplification" {
                                        0.55
                                    } else {
                                        0.5
                                    }
                                } else if focused && !self.decisive {
                                    0.55
                                } else {
                                    1.0
                                }
                            } else if !focused
                                && name == "function_simplification"
                                && key == "clear"
                            {
                                0.45
                            } else if !focused && probe && key == "clear" {
                                0.5
                            } else if focused && !self.decisive && key == "clear" {
                                0.45
                            } else {
                                0.0
                            };
                            (key.clone(), json!(probability))
                        })
                        .collect::<serde_json::Map<_, _>>();
                    answers.insert(name.clone(), json!({"type":"choice","choice":chosen,"confidence":0.4,"probabilities":probabilities}));
                }
                Ok(
                    json!({"model":request["model"],"answers":answers,"usage":{"input_tokens":10,"output_tokens":4}}),
                )
            }
        }
        let mut decisive = Focus {
            calls: 0,
            source: String::new(),
            decisive: true,
        };
        let resolved = run(&project, &options, &mut decisive);
        assert_eq!(
            decisive.calls,
            2,
            "file {:?} report {:?}",
            resolved.files.first().map(|file| file.error.clone()),
            resolved.errors
        );
        assert!(decisive.source.contains("load_inputs"));
        assert!(!decisive.source.contains("keep_ready"));
        assert_eq!(
            resolved.files[0].dimensions["function_simplification"].status,
            Status::Clear
        );
        assert_eq!(
            resolved.files[0].dimensions["function_simplification"].concern_basis,
            "focused-evidence"
        );
        assert!(resolved.files[0].findings.is_empty());
        let mut split = Focus {
            calls: 0,
            source: String::new(),
            decisive: false,
        };
        let unresolved = run(&project, &options, &mut split);
        assert_eq!(split.calls, 2);
        assert_eq!(
            unresolved.files[0].dimensions["function_simplification"].status,
            Status::Uncertain
        );
        assert!(unresolved.files[0].findings.is_empty());
    }
}
