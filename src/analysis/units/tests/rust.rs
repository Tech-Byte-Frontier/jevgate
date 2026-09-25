//! Rust: types, methods of their `impl`, functions and imports.
use super::*;

const STORE: &str = "use std::path::PathBuf;\n\n/// Stored records.\npub struct Store { root: PathBuf }\n\nimpl Store {\n    /// Opens the store.\n    pub fn open(root: PathBuf, strict: bool) -> Self {\n        if strict {\n            for _ in 0..2 {\n                check(&root);\n            }\n        }\n        let store = Self { root };\n        store.touch();\n        store\n    }\n    fn touch(&self) {}\n}\n\nfn check(path: &PathBuf) {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn opens() {}\n}\n";

fn store_unit(name: &str) -> Unit {
    parse(Path::new("store.rs"), STORE)
        .unwrap()
        .units
        .into_iter()
        .find(|u| u.name == name)
        .unwrap()
}

#[test]
fn rust_units_are_types_methods_and_functions() {
    assert_eq!(
        names("store.rs", STORE),
        [
            ("Store".into(), Kind::Type),
            ("Store::open".into(), Kind::Method),
            ("Store::touch".into(), Kind::Method),
            ("check".into(), Kind::Function),
            ("opens".into(), Kind::Function),
        ]
    );
}

#[test]
fn a_rust_method_keeps_its_signature_doc_and_leading_comment() {
    let open = store_unit("Store::open");
    assert_eq!(
        open.signature,
        "pub fn open(root: PathBuf, strict: bool) -> Self"
    );
    assert_eq!(open.doc, "Opens the store.");
    assert!(open.source(STORE).starts_with("/// Opens"));
}

#[test]
fn rust_imports_are_recorded() {
    assert!(
        parse(Path::new("store.rs"), STORE)
            .unwrap()
            .imports
            .contains("PathBuf")
    );
}

#[test]
fn empty_bodies_are_too_small() {
    assert!(!store_unit("Store::open").too_small());
    assert!(store_unit("Store::touch").too_small());
}
