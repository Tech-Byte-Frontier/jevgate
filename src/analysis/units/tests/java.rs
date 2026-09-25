//! Java: methods of their class, interface, enum constant or record.
use super::*;

const OWNERS: &str = "package app.owner;\n\nimport java.util.*;\nimport java.util.List;\nimport static app.Checks.requireName;\n\n/** Owners of pets. */\npublic class OwnerService {\n\n\tprivate static final String DEFAULT_CITY = \"Madison\";\n\n\tstatic int retries = 5;\n\n\tprivate final int pageSize = 25;\n\n\tprivate final OwnerRepository owners;\n\n\tpublic OwnerService(OwnerRepository owners) {\n\t\tthis.owners = owners;\n\t}\n\n\t/**\n\t * Finds owners by last name.\n\t */\n\t@Transactional\n\tpublic List<Owner> find(String name, int page) {\n\t\tif (name == null) {\n\t\t\tthrow new IllegalArgumentException(\"name is required\");\n\t\t} else if (name.isBlank()) {\n\t\t\treturn new ArrayList<>();\n\t\t} else if (page > 100) {\n\t\t\treturn List.of();\n\t\t}\n\t\tString query = String.format(\"last_name = '%s'\", name);\n\t\tfor (Owner owner : owners.findAll(query)) {\n\t\t\towner.getPets().forEach(pet -> {\n\t\t\t\tswitch (pet.getKind()) {\n\t\t\t\t\tcase \"cat\" -> requireName(pet);\n\t\t\t\t\tdefault -> log(\"skipped \" + pet.getName());\n\t\t\t\t}\n\t\t\t});\n\t\t}\n\t\treturn owners.page(query, page * 3);\n\t}\n\n\t@Override\n\tpublic boolean equals(Object other) {\n\t\tif (this == other) return true;\n\t\tif (!(other instanceof OwnerService)) return false;\n\t\tOwnerService that = (OwnerService) other;\n\t\treturn owners.equals(that.owners);\n\t}\n\n\t@Override\n\tpublic int hashCode() {\n\t\tint result = 17;\n\t\tresult = 31 * result + owners.hashCode();\n\t\tresult = 31 * result + 7;\n\t\treturn result;\n\t}\n\n\tstatic class Page {\n\t\tint size() { return 10; }\n\t}\n}\n\ninterface OwnerRepository {\n\tString TABLE = \"owners\";\n\n\tList<Owner> findAll(String query);\n\n\tdefault List<Owner> page(String query, int size) {\n\t\treturn findAll(query).subList(0, size);\n\t}\n}\n\nenum State {\n\tOPEN {\n\t\tvoid enter(Owner owner) {\n\t\t\towner.open();\n\t\t}\n\t},\n\tCLOSED;\n\n\tprivate static final int LIMIT = 42;\n\n\tvoid enter(Owner owner) {}\n}\n\nrecord Visit(String date, String description) {\n\tVisit {\n\t\trequireName(description);\n\t}\n}\n\n@interface Audited {}\n";

#[test]
fn java_methods_belong_to_their_class_interface_enum_constant_or_record() {
    let file = parse(Path::new("OwnerService.java"), OWNERS).unwrap();
    let named: Vec<(&str, Kind, usize)> = file
        .units
        .iter()
        .map(|u| (u.name.as_str(), u.kind, u.line))
        .collect();
    assert_eq!(
        named,
        [
            ("OwnerService::OwnerService", Kind::Method, 18),
            ("OwnerService::find", Kind::Method, 25),
            ("OwnerService::equals", Kind::Method, 46),
            ("OwnerService::hashCode", Kind::Method, 54),
            ("Page::size", Kind::Method, 63),
            ("OwnerRepository::findAll", Kind::Method, 70),
            ("OwnerRepository::page", Kind::Method, 72),
            ("OPEN::enter", Kind::Method, 79),
            ("State::enter", Kind::Method, 87),
            ("Visit::Visit", Kind::Method, 91),
            ("Audited", Kind::Type, 96),
        ]
    );
    assert!(crate::syntax::supported(Path::new("OwnerService.java")));
    // A wildcard import names no class; a static import names its member.
    let imports: Vec<&str> = file.imports.iter().map(String::as_str).collect();
    assert_eq!(imports, ["List", "requireName"]);
}

#[test]
fn a_java_method_has_its_calls_nesting_values_sites_and_errors() {
    let file = parse(Path::new("OwnerService.java"), OWNERS).unwrap();
    let find = &file.units[1];
    assert_eq!(
        find.signature,
        "public List<Owner> find(String name, int page)"
    );
    assert_eq!(find.doc, "Finds owners by last name.");
    assert!(find.source(OWNERS).starts_with("/**\n\t * Finds"));
    for call in [
        "findAll",
        "forEach",
        "requireName",
        "format",
        "ArrayList",
        "page",
    ] {
        assert!(find.calls.contains(call), "{call}");
    }
    assert!(find.refs.contains("Owner") && find.refs.contains("OwnerService"));
    let literals: Vec<&str> = find.literals.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        literals,
        [
            "\"name is required\"",
            "100",
            "\"last_name = '%s'\"",
            "\"cat\"",
            "\"skipped \"",
            "3"
        ]
    );
    let sites: Vec<&str> = find.sites.iter().map(|s| s.text.as_str()).collect();
    assert!(
        sites.contains(&"String query = String.format(\"last_name = '%s'\", name);"),
        "{sites:?}"
    );
    assert!(
        sites.contains(&"log(\"skipped \" + pet.getName());"),
        "{sites:?}"
    );
    // The else-if chain is one level; the loop, lambda block and switch nest.
    assert_flow(
        find,
        (3, 3),
        &[("IllegalArgumentException", "\"name is required\"")],
    );
    assert_eq!(find.blocks.len(), 3);
    // Static fields and interface constants are constants; instance fields are not.
    assert_eq!(
        constant_names(&file),
        ["DEFAULT_CITY", "retries", "TABLE", "LIMIT"]
    );
}

#[test]
fn java_capacity_hints_and_numbers_a_method_returns_are_not_values() {
    let java = "class Costs {\n\tint cost() {\n\t\treturn 7;\n\t}\n\n\tString host() {\n\t\treturn \"db.internal\";\n\t}\n\n\tList<String> names() {\n\t\tList<String> names = new ArrayList<>(16);\n\t\tStringBuilder text = new StringBuilder(64);\n\t\tnames.add(text.append(new Timeout(30)).toString());\n\t\treturn names.subList(0, 5);\n\t}\n}\n";
    let file = parse(Path::new("Costs.java"), java).unwrap();
    let values: Vec<(&str, Vec<&str>)> = file
        .units
        .iter()
        .map(|u| {
            (
                u.name.as_str(),
                u.literals.iter().map(|l| l.text.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        values,
        [
            ("Costs::cost", vec![]),
            ("Costs::host", vec!["\"db.internal\""]),
            ("Costs::names", vec!["30", "5"]),
        ]
    );
}

#[test]
fn java_equals_and_hash_code_offer_no_values() {
    let file = parse(Path::new("OwnerService.java"), OWNERS).unwrap();
    let equality: Vec<&str> = file
        .units
        .iter()
        .filter(|u| u.equality)
        .map(|u| u.name.as_str())
        .collect();
    assert_eq!(equality, ["OwnerService::equals", "OwnerService::hashCode"]);
    assert!(file.units[3].literals.is_empty());
}
