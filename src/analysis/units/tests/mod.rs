//! Units of each supported language, one file per language; what every
//! language measures the same way is here.
mod csharp;
mod go;
mod java;
mod javascript;
mod php;
mod python;
mod ruby;
mod rust;

use super::*;

fn names(path: &str, source: &str) -> Vec<(String, Kind)> {
    parse(Path::new(path), source)
        .unwrap()
        .units
        .into_iter()
        .map(|u| (u.name, u.kind))
        .collect()
}

/// A unit's created errors, as (error, message).
fn created_errors(unit: &Unit) -> Vec<(&str, &str)> {
    unit.errors
        .iter()
        .map(|e| (e.error.as_str(), e.message.as_str()))
        .collect()
}

/// Each of `names` is a name the file's imports record.
fn assert_imports(file: &FileUnits, names: &[&str]) {
    for name in names {
        assert!(file.imports.contains(*name), "{name}");
    }
}

/// A judged unit's control flow (its nesting, then its branch chain) and the
/// errors it creates, which every language's parser records the same way.
fn assert_flow(unit: &Unit, nesting: (usize, usize), errors: &[(&str, &str)]) {
    assert_eq!((unit.nesting, unit.branch_chain), nesting, "{}", unit.name);
    assert_eq!(created_errors(unit), errors, "{}", unit.name);
}

/// A file's constants, by name.
fn constant_names(file: &FileUnits) -> Vec<&str> {
    file.constants.iter().map(|c| c.name.as_str()).collect()
}

#[test]
fn nesting_and_branch_chains_are_measured_without_counting_else_if_as_depth() {
    let facts = |path: &str, source: &str| {
        let unit = parse(Path::new(path), source).unwrap().units.remove(0);
        (unit.nesting, unit.branch_chain, unit.deeply_nested())
    };
    let ternary = "function icon(kind) {\n  const name = kind === 'a' ? 'x' : kind === 'b' ? 'y' : kind === 'c' ? 'z' : kind === 'd' ? 'w' : 'v'\n  return name\n}\n";
    assert_eq!(facts("a.ts", ternary), (1, 5, true));
    let chain = "function f(x) {\n  if (x === 1) return 1\n  else if (x === 2) return 2\n  else return 3\n}\n";
    assert_eq!(facts("b.ts", chain), (1, 3, false));
    let deep = "fn f(xs: &[Vec<u8>]) {\n    for x in xs {\n        if x.len() > 1 {\n            for y in x {\n                if *y > 2 {\n                    println!(\"{y}\");\n                }\n            }\n        }\n    }\n}\n";
    assert_eq!(facts("c.rs", deep), (4, 1, true));
    let python = "def f(x):\n    if x == 1:\n        return 1\n    elif x == 2:\n        return 2\n    elif x == 3:\n        return 3\n    else:\n        return 4\n";
    assert_eq!(facts("d.py", python), (1, 4, true));
    let open = "def f(x):\n    if x == 1:\n        return 1\n    elif x == 2:\n        return 2\n    return 3\n";
    assert_eq!(facts("e.py", open), (1, 2, false));
}

#[test]
fn unsupported_languages_are_unparsed() {
    assert!(!parse(Path::new("Main.kt"), "class Main {}").unwrap().parsed);
}

#[test]
fn syntax_errors_fail_instead_of_returning_no_units() {
    assert!(parse(Path::new("broken.rs"), "fn broken( {").is_err());
}

/// Valid TypeScript that tree-sitter-typescript misreads: a call signature
/// starting with `<T>` on the line after another reads as its continuation.
const SIGNATURES: &str = "type CreateStore = {\n  <T>(initializer: Init<T>): Store<T>\n\n  <T>(): (initializer: Init<T>) => Store<T>\n}\n\nexport const createStore = (initializer: unknown) => {\n  const listeners = new Set<() => void>()\n  const subscribe = (listener: () => void) => {\n    listeners.add(listener)\n    return () => listeners.delete(listener)\n  }\n  return { subscribe, initializer }\n}\n";

#[test]
fn a_grammar_gap_leaves_the_rest_of_a_file_to_judge() {
    let units = parse(Path::new("vanilla.ts"), SIGNATURES).unwrap();
    let names: Vec<&str> = units.units.iter().map(|u| u.name.as_str()).collect();
    assert!(names.contains(&"createStore"), "{names:?}");
    assert!(
        !names.contains(&"CreateStore"),
        "the misread type is left out"
    );
    // A function holding the error is left out too.
    let inside = "export function create() {\n  type Api = {\n    <T>(a: T): T\n\n    <T>(): () => T\n  }\n  return 1\n}\n\nexport function other(values: number[]) {\n  let total = 0\n  for (const value of values) {\n    total += value\n  }\n  return total\n}\n";
    let names: Vec<String> = parse(Path::new("api.ts"), inside)
        .unwrap()
        .units
        .into_iter()
        .map(|u| u.name)
        .collect();
    assert_eq!(names, ["other"]);
    // A generator template's placeholders keep it unjudged.
    assert!(parse(Path::new("lib/templates/store.ts"), SIGNATURES).is_err());
    let erb = format!("// <%= banner %>\n{SIGNATURES}");
    assert!(parse(Path::new("store.ts"), &erb).is_err());
}
