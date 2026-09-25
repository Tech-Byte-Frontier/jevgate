//! Go: functions, methods of their receiver's type, and types.
use super::*;

#[test]
fn go_functions_methods_types_and_their_facts_are_units() {
    let source = "package store\n\nimport (\n\t\"database/sql\"\n\t\"fmt\"\n)\n\nconst maxRows = 500\n\n// Store keeps users.\ntype Store struct {\n\tdb *sql.DB\n}\n\n// Find loads a user.\nfunc (s *Store) Find(name string) (*User, error) {\n\tq := fmt.Sprintf(\"SELECT * FROM users WHERE name = '%s'\", name)\n\tif name == \"\" {\n\t\treturn nil, fmt.Errorf(\"empty name: %w\", ErrInvalid)\n\t} else if name == \"root\" {\n\t\treturn nil, errors.New(\"reserved\")\n\t}\n\tfor i := 0; i < 3; i++ {\n\t\tswitch i {\n\t\tcase 1:\n\t\t\ts.db.Query(q)\n\t\t}\n\t}\n\treturn nil, nil\n}\n\nfunc New(db *sql.DB) *Store { return &Store{db: db} }\n";
    let file = parse(Path::new("store.go"), source).unwrap();
    let named: Vec<(&str, &str, usize)> = file
        .units
        .iter()
        .map(|u| (u.name.as_str(), u.owner.as_str(), u.line))
        .collect();
    assert_eq!(
        named,
        [
            ("Store", "", 11),
            ("Store::Find", "Store", 16),
            ("New", "", 32)
        ]
    );
    let find = &file.units[1];
    assert_imports(&file, &["fmt", "sql"]);
    assert!(find.calls.contains("Sprintf") && find.calls.contains("Query"));
    assert_eq!((find.nesting, find.branch_chain), (2, 2));
    assert_eq!(
        find.sites[0].text,
        "q := fmt.Sprintf(\"SELECT * FROM users WHERE name = '%s'\", name)"
    );
    let errors: Vec<&str> = find.errors.iter().map(|e| e.error.as_str()).collect();
    assert_eq!(errors, ["fmt.Errorf", "errors.New"]);
    assert!(find.literals.iter().any(|l| l.text == "\"root\""));
    assert_eq!(file.constants[0].name, "maxRows");
    assert!(crate::syntax::supported(Path::new("store.go")));
}
