use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
pub struct Rule {
    pub id: &'static str,
    pub key: &'static str,
    pub version: &'static str,
    pub scope: &'static str,
    pub required_context: &'static str,
    pub inspection: &'static str,
    pub acceptable_example: &'static str,
    pub evaluation_dataset: &'static str,
    pub thresholds_validated: bool,
}

pub fn rules() -> Vec<Rule> {
    crate::maintainability::rules()
}
pub fn rule_version(key: &str) -> &'static str {
    match key {
        "file_organization" => "8",
        "function_simplification" => "7",
        "shared_logic" => "15",
        _ => "7",
    }
}
pub fn describe() -> Value {
    Value::Array(
        rules()
            .into_iter()
            .map(|r| {
                let mut value = serde_json::to_value(r).unwrap();
                value["decision_policy"] = serde_json::json!(crate::maintainability::policy());
                value
            })
            .collect(),
    )
}
