//! Units of each supported language, one file per language; what every
//! language measures the same way is here.
mod bend;
mod csharp;
mod generic;
mod go;
mod java;
mod javascript;
mod navigation;
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
    assert!(
        !parse(Path::new("main.zig"), "pub fn main() void {}")
            .unwrap()
            .parsed
    );
}

#[test]
fn a_file_the_parser_could_not_read_fails_instead_of_returning_no_units() {
    // Bend 2's grammar reads no top level here: a `match` inside `do`.
    let unread = "def main() -> IO(Unit):\n  do IO<Unit>:\n    match b:\n      case True{}:\n        IO.print(\"t\")\n";
    assert!(parse(Path::new("main.bend"), unread).is_err());
    // A top level of nothing but an error leaves no unit, and says so.
    let broken = parse(Path::new("broken.rs"), "fn broken( {").unwrap();
    assert!(broken.units.is_empty() && broken.partial());
    assert_eq!(
        broken.left_out_code("fn broken( {"),
        [LeftOut {
            name: String::new(),
            span: 0..12,
            line: 1,
            end_line: 1,
            error_line: 1,
        }]
    );
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
    let left_out: Vec<(&str, usize, usize)> = units
        .left_out
        .iter()
        .map(|l| (l.name.as_str(), l.line, l.error_line))
        .collect();
    assert_eq!(left_out, [("CreateStore", 1, 2)]);
    // A function holding the error is left out too.
    let inside = "export function create() {\n  type Api = {\n    <T>(a: T): T\n\n    <T>(): () => T\n  }\n  return 1\n}\n\nexport function other(values: number[]) {\n  let total = 0\n  for (const value of values) {\n    total += value\n  }\n  return total\n}\n";
    let names: Vec<String> = parse(Path::new("api.ts"), inside)
        .unwrap()
        .units
        .into_iter()
        .map(|u| u.name)
        .collect();
    assert_eq!(names, ["other"]);
    // A generator template's placeholders keep it unjudged: under a
    // templates directory, or an ERB tag in its code.
    assert!(parse(Path::new("lib/templates/store.ts"), SIGNATURES).is_err());
    let erb = format!("export const <%= name %> = 1\n{SIGNATURES}");
    assert!(parse(Path::new("store.ts"), &erb).is_err());
}

/// A function tree-sitter-rust misreads: it takes snapbox's `str![…]` for
/// the type `str`, one error in each (12 of mdbook's test files).
fn snapshot(name: &str) -> String {
    format!(
        "/// Reads the {name} snapshot.\nfn {name}() -> usize {{\n    let text = str![[\"{name}\"]];\n    text.len()\n}}\n"
    )
}

#[test]
fn any_number_of_errors_leaves_out_only_the_units_holding_them() {
    // Five errors: more than the three regions a file could hold before.
    let broken: String = ["a", "b", "c", "d", "e"].map(snapshot).concat();
    let source = format!(
        "{}{broken}{}",
        crate::tests::function("first"),
        crate::tests::function("last")
    );
    let file = parse(Path::new("src/snapshots.rs"), &source).unwrap();
    let kept: Vec<&str> = file.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(kept, ["first", "last"]);
    let left_out: Vec<(&str, usize, usize, usize)> = file
        .left_out
        .iter()
        .map(|l| (l.name.as_str(), l.line, l.end_line, l.error_line))
        .collect();
    assert_eq!(
        left_out,
        [
            ("a", 10, 13, 11),
            ("b", 15, 18, 16),
            ("c", 20, 23, 21),
            ("d", 25, 28, 26),
            ("e", 30, 33, 31),
        ]
    );
    // A left-out function's documentation goes with it.
    let doc = source.find("/// Reads the a snapshot").unwrap();
    assert!(!file.intact(&(doc..doc + 3)));
    let first = &file.units[0];
    assert!(file.intact(&first.span));
    // The five functions with their documentation: 25 of 41 non-blank lines.
    assert!((file.coverage(&source) - 16.0 / 41.0).abs() < 1e-9);
}

#[test]
fn an_error_outside_every_unit_is_left_out_by_its_lines() {
    let source = "import os\n\nLIMIT = 3\nBROKEN = )\n\ndef total(values):\n    result = 0\n    for value in values:\n        result += value\n    return result\n\nos.environ.setdefault(\"A\", \"b\"))\nos.environ.setdefault(\"C\", \"d\")\n";
    let file = parse(Path::new("app/settings.py"), source).unwrap();
    assert_eq!(file.units.len(), 1);
    assert!(file.left_out.is_empty() && file.partial());
    let code: Vec<(usize, usize, usize)> = file
        .left_out_code(source)
        .iter()
        .map(|l| (l.line, l.end_line, l.error_line))
        .collect();
    assert_eq!(code, [(4, 4, 4), (12, 12, 12)]);
    // Constants and statements holding an error are not judged; the rest is.
    let constants: Vec<&str> = file.constants.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(constants, ["LIMIT"]);
    let statements: Vec<usize> = file.setup.statements.iter().map(|s| s.1).collect();
    assert_eq!(statements, [13]);
}

#[test]
fn a_type_whose_members_are_left_out_is_not_judged_whole() {
    let source = format!(
        "struct Store;\n\nimpl Store {{\n{}}}\n",
        snapshot("read").replace('\n', "\n    ")
    );
    let file = parse(Path::new("store.rs"), &source).unwrap();
    let kept: Vec<&str> = file.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(kept, ["Store"]);
    let left_out: Vec<&str> = file.left_out.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(left_out, ["Store::read"]);
    let class = "class Store:\n    def read(self):\n        return str![[1]]\n";
    let file = parse(Path::new("store.py"), class).unwrap();
    assert!(file.units.is_empty(), "{:?}", file.units);
}
