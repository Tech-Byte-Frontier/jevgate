//! The JSON Schema of `jevgate.toml`, generated from the configuration types
//! with the rule names, gate levels and limits filled in. `jevgate.schema.json`
//! at the repository root holds it, and `jevgate init` points editors to it.
use crate::{catalog, config::Config, options::MAX_CONCURRENCY};
use serde_json::{Value, json};

const ID: &str =
    "https://raw.githubusercontent.com/Tech-Byte-Frontier/jevgate/main/jevgate.schema.json";
/// Levels `fail_on` accepts; `[rules]` also accepts `off`.
const LEVELS: [&str; 6] = [
    "review",
    "consider",
    "mature",
    "uncertain",
    "report",
    "none",
];

pub fn schema() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(Config)).expect("a schema is JSON");
    tomlify(&mut schema);
    schema["$id"] = json!(ID);
    schema["title"] = json!("jevgate.toml");
    schema["description"] = json!(
        "JevGate configuration. The command line wins over the file, except that upload patterns and budgets in the file are ceilings that flags can only narrow. Unknown keys are errors."
    );
    bound_budgets(&mut schema["properties"]);
    list_levels(&mut schema);
    list_names(&mut schema["$defs"]);
    crate::custom::schema(&mut schema["$defs"]);
    schema
}

/// The least and most each budget accepts.
fn bound_budgets(properties: &mut Value) {
    properties["concurrency"]["minimum"] = json!(1);
    properties["concurrency"]["maximum"] = json!(MAX_CONCURRENCY);
    for budget in ["max_requests", "max_file_bytes", "max_context_bytes"] {
        properties[budget]["minimum"] = json!(1);
    }
}

/// The gate levels `fail_on` and `[[scope]]` accept, and with `off` the
/// levels of `[rules]`.
fn list_levels(schema: &mut Value) {
    let levels = json!(LEVELS);
    let mut with_off = LEVELS.to_vec();
    with_off.push("off");
    schema["properties"]["fail_on"]["items"]["enum"] = levels.clone();
    let definitions = &mut schema["$defs"];
    definitions["Scope"]["properties"]["fail_on"]["items"]["enum"] = levels;
    for level in definitions["Level"]["anyOf"].as_array_mut().unwrap() {
        match level["type"].as_str() {
            Some("string") => level["enum"] = json!(with_off),
            _ => level["items"]["enum"] = json!(with_off),
        }
    }
}

/// The rule names `rules`, `[rules]` and `[[scope]]` accept: every built-in
/// name, and a custom question's ID, which only the configuration knows.
fn list_names(definitions: &mut Value) {
    let named = json!({"anyOf": [{"enum": rule_names()}, {"pattern": crate::custom::NAMES}]});
    for rules in definitions["Rules"]["anyOf"].as_array_mut().unwrap() {
        match rules["type"].as_str() {
            Some("array") => rules["items"] = json!({"type": "string", "anyOf": named["anyOf"]}),
            _ => rules["propertyNames"] = named.clone(),
        }
    }
    definitions["Scope"]["properties"]["rules"]["propertyNames"] = named;
}

/// Every rule ID, name, key and group, and the `default` and `all` groups.
fn rule_names() -> Value {
    let rules = catalog::rules();
    let mut names: Vec<&str> = Vec::new();
    for rule in &rules {
        let short = rule.id.rsplit_once('/').map_or(rule.id, |(_, short)| short);
        for name in [rule.id, short, rule.key] {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names.extend(catalog::groups());
    names.extend([catalog::DEFAULT_GROUP, catalog::ALL_GROUP]);
    json!(names)
}

/// TOML has no null and the descriptions state each default, so optional
/// values become plain types and generated defaults and formats are dropped.
fn tomlify(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("default");
            map.remove("format");
            if let Some(Value::Array(types)) = map.get_mut("type") {
                types.retain(|t| t != "null");
                if let [single] = types.as_slice() {
                    let single = single.clone();
                    map.insert("type".into(), single);
                }
            }
            // An optional enum is one of its schema or null.
            if let Some(Value::Array(options)) = map.get_mut("anyOf") {
                options.retain(|option| *option != json!({"type": "null"}));
                if let [Value::Object(single)] = options.as_slice() {
                    let single = single.clone();
                    map.remove("anyOf");
                    map.extend(single);
                }
            }
            map.values_mut().for_each(tomlify);
        }
        Value::Array(items) => items.iter_mut().for_each(tomlify),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/jevgate.schema.json");

    /// The checked-in schema matches the configuration types. After changing
    /// them, rerun with `JEVGATE_WRITE_SCHEMA=1` to rewrite the file.
    #[test]
    fn checked_in_schema_matches_the_configuration() {
        if crate::tests::packaged() {
            return;
        }
        let text = serde_json::to_string_pretty(&schema()).unwrap() + "\n";
        if std::env::var_os("JEVGATE_WRITE_SCHEMA").is_some() {
            std::fs::write(FILE, &text).unwrap();
        }
        // Git may check the file out with CRLF line ends on Windows.
        let file = std::fs::read_to_string(FILE)
            .unwrap_or_default()
            .replace('\r', "");
        assert!(
            file == text.replace('\r', ""),
            "jevgate.schema.json is out of date; run JEVGATE_WRITE_SCHEMA=1 cargo test"
        );
    }

    #[test]
    fn rule_names_and_levels_are_listed() {
        let schema = schema();
        let named = &schema["$defs"]["Scope"]["properties"]["rules"]["propertyNames"];
        let names = named["anyOf"][0]["enum"].as_array().unwrap();
        assert_eq!(named["anyOf"][1]["pattern"], crate::custom::NAMES);
        let question = &schema["$defs"]["Question"];
        assert_eq!(question["required"], json!(["question", "unit"]));
        assert_eq!(question["properties"]["threshold"]["maximum"], 0.99);
        let example = &schema["$defs"]["QuestionExample"];
        assert_eq!(example["dependentRequired"], json!({"code": ["path"]}));
        for name in [
            "security",
            "security/injection",
            "injection",
            "default",
            "all",
        ] {
            assert!(names.contains(&json!(name)), "{name}");
        }
        assert_eq!(schema["properties"]["max_requests"]["type"], "integer");
        assert!(schema["properties"]["fail_on"].get("default").is_none());
    }
}
