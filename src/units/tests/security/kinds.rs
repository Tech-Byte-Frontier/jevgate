//! The kinds of injection a trace checks: redirects, uploads, deserializers and
//! XML parsers asked about only where the source names one, types named by
//! input, and the enums a trace shows.
use super::*;

#[test]
fn redirects_deserializers_and_uploads_are_checked_kinds_with_their_weakness() {
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    for (check, category) in [
        ("redirect", "CWE-601 open redirect"),
        ("deserialize", "CWE-502 deserialization of untrusted data"),
        ("upload", "CWE-434 unrestricted file upload"),
    ] {
        let project = Project::new();
        project.write(
            "go.php",
            "<?php\n$target = $_GET['next'];\nheader('Location: ' . $target);\n$prefs = unserialize($_COOKIE['prefs']);\nmove_uploaded_file($_FILES['f']['tmp_name'], 'up/' . $_FILES['f']['name']);\n",
        );
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("resource", noul_at(0.95)),
            (check, noul_at(0.95)),
            ("origin", spread(0.0, 0.05, 0.95)),
            ("redirect_reach", choice_of("anywhere", &REACH)),
        ];
        let report = run(&project, &options, &mut eval);
        let finding = &report.files[0].findings[0];
        assert_eq!(finding.category.as_deref(), Some(category));
        assert_eq!(finding.strength, Strength::Review);
    }
}

/// The injection plan of a project holding only `source` at `path`.
pub(super) fn injection_plan(path: &str, source: &str) -> Plan {
    let project = Project::new();
    project.write(path, source);
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    planned(&project, &options).1
}

/// The questions of the trace planned for the first unit of `path`.
pub(super) fn traced_checks(path: &str, source: &str) -> serde_json::Map<String, Value> {
    let plan = injection_plan(path, source);
    let trace = &plan.files[&0].units[0];
    let Detail::Security {
        trace: Some(trace), ..
    } = &trace.detail
    else {
        panic!("no trace planned");
    };
    trace.request()["questions"].as_object().unwrap().clone()
}

pub(super) const PICKLED: &str = "import pickle\n\nfrom flask import jsonify, request\n\n\ndef restore_cart():\n    cart = pickle.loads(request.get_data())\n    return jsonify(items=len(cart))\n";

#[test]
fn a_deserializer_is_asked_about_only_where_the_source_names_one() {
    let first = |path: &str, source: &str| {
        injection_plan(path, source).requests[0].request["questions"]["f0_interpreted"].clone()
    };
    let named = first("shop/cart.py", PICKLED);
    assert!(named.to_string().contains("pickle, marshal"), "{named}");
    let parsed = PICKLED
        .replace("import pickle", "import json")
        .replace("pickle.loads", "json.loads");
    assert_eq!(
        first("shop/cart.py", &parsed),
        questions::security_interpreted("functions[0].source", false, false, None, false),
        "code that names no deserializer keeps its question and cached answer"
    );
    assert!(traced_checks("shop/cart.py", PICKLED).contains_key("deserialize"));
    assert!(!traced_checks("shop/cart.py", &parsed).contains_key("deserialize"));
    let ruby = traced_checks(
        "app/models/cart.rb",
        "class Cart\n  def self.restore(params)\n    Marshal.load(Base64.decode64(params[:cart]))\n  end\nend\n",
    );
    assert!(ruby["deserialize"].to_string().contains("Marshal.load"));
    let java = traced_checks(
        "src/main/java/shop/Cart.java",
        "class Cart {\n  Object restore(InputStream body) throws Exception {\n    return new ObjectInputStream(body).readObject();\n  }\n}\n",
    );
    assert!(
        java["deserialize"]
            .to_string()
            .contains("ObjectInputStream")
    );
}

#[test]
fn request_data_given_to_pickle_is_a_deserialization_review() {
    let project = Project::new();
    project.write("shop/cart.py", PICKLED);
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.95)),
        ("deserialize", noul_at(0.95)),
        ("origin", spread(0.0, 0.05, 0.95)),
    ];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-502 deserialization of untrusted data")
    );
    assert_eq!(finding.strength, Strength::Review);
}

pub(super) const XML_IMPORT: &str = "import java.io.InputStream;\nimport javax.xml.parsers.DocumentBuilderFactory;\n\nclass Catalog {\n  int count(InputStream body) throws Exception {\n    var document = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(body);\n    return document.getElementsByTagName(\"item\").getLength();\n  }\n}\n";

#[test]
fn xml_parsed_with_entities_is_asked_about_only_where_a_parser_is_named() {
    let path = "src/main/java/shop/Catalog.java";
    let named = traced_checks(path, XML_IMPORT);
    assert!(named["xxe"].to_string().contains("DocumentBuilderFactory"));
    let plain = XML_IMPORT
        .replace("import javax.xml.parsers.DocumentBuilderFactory;\n", "")
        .replace(
            "DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(body)",
            "Json.parse(body)",
        );
    assert!(!traced_checks(path, &plain).contains_key("xxe"));
    // A module that imports the parser at its top, as Python code does.
    let module = "from lxml import etree\n\n\ndef count(body):\n    root = etree.fromstring(body)\n    return len(root.findall('item'))\n";
    assert!(traced_checks("shop/catalog.py", module).contains_key("xxe"));
    let project = Project::new();
    project.write(path, XML_IMPORT);
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = scripted(0);
    eval.overrides = vec![
        ("interpreted", noul_at(0.95)),
        ("xxe", noul_at(0.95)),
        ("origin", spread(0.0, 0.05, 0.95)),
    ];
    let report = run(&project, &options, &mut eval);
    let finding = &report.files[0].findings[0];
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-611 XML external entity reference")
    );
    assert_eq!(finding.strength, Strength::Review);
}

#[test]
fn an_injection_trace_shows_the_enums_its_sites_name() {
    let project = Project::new();
    project.write(
        "server/store.ts",
        "import { ConfigKey } from '../shared/config'\n\nexport async function setAITagConfig(DB: D1Database, config: AITagConfig): Promise<boolean> {\n  const insertSql = `INSERT INTO stores (key, value) VALUES ('${ConfigKey.aiTag}', ?) ON CONFLICT(key) DO UPDATE SET value = ?`\n  const bindValue = JSON.stringify(config)\n  const result = await DB.prepare(insertSql).bind(bindValue, bindValue).run()\n  return result.success\n}\n",
    );
    project.write(
        "shared/config.ts",
        "enum ConfigKey {\n  shouldShowRecent = 'config/should_show_recent',\n  aiTag = 'config/ai_tag',\n}\n\nexport { ConfigKey }\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let (_, plan) = planned(&project, &options);
    let traces: Vec<Value> = plan
        .files
        .values()
        .flat_map(|f| &f.units)
        .filter_map(|u| match &u.detail {
            Detail::Security {
                trace: Some(trace), ..
            } => Some(trace.request()),
            _ => None,
        })
        .collect();
    assert_eq!(traces.len(), 1);
    assert!(
        traces[0]["state"]["enums_named_in_sites"][0]
            .as_str()
            .is_some_and(|e| e.starts_with("enum ConfigKey {")),
        "{}",
        traces[0]["state"]
    );
}

#[test]
fn a_csharp_type_named_by_input_is_an_injection_named_by_its_own_check() {
    let project = Project::new();
    project.write(
        "Controllers/ImportsController.cs",
        "namespace Api;\n\npublic class ImportsController : Controller\n{\n    [HttpPost]\n    public IActionResult Post(string typeName, string xml)\n    {\n        var serializer = new XmlSerializer(Type.GetType(typeName));\n        return Ok(serializer.Deserialize(new StringReader(xml)));\n    }\n}\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let mut eval = recording(&[("interpreted", 0.95), ("type", 0.95)]);
    eval.inner
        .overrides
        .push(("origin", spread(0.0, 0.05, 0.95)));
    let report = run(&project, &options, &mut eval);
    let first = &eval.requests[0]["questions"]["f0_interpreted"];
    assert!(
        first["instructions"]["question"]
            .as_str()
            .unwrap()
            .ends_with("or into the type of objects it creates?"),
        "{first}"
    );
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-502 deserialization of untrusted data")
    );
    assert!(
        finding.message.contains("types of objects it creates"),
        "{}",
        finding.message
    );
}
