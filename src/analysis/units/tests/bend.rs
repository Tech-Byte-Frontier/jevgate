//! Bend 2: defs, types and laws; proofs, type-level defs and effects; calls
//! through import aliases; laws read in words; tests that end in their output.
use super::*;
use crate::analysis::units::{Kind, Role};

const SORT: &str = "# Sorting, with the laws it keeps.\nimport Base\nimport ./main.bend as Sort\n\ntype Shape is Data:\n  Circle{r: U32}\n  Square{s: U32}\n\n# LE(a, b): the proofs that a <= b\ndef LE(a: Nat, b: Nat) -> Data:\n  match a b:\n    case 0n b0:\n      Unit\n    case 1n+a1 0n:\n      Empty\n    case 1n+a1 1n+b1:\n      LE(a1, b1)\n\ndef area(x: Shape) -> U32:\n  match x:\n    case Circle{+r}:\n      (3 * r * r : U32)\n    case Square{+s}:\n      (s * s : U32)\n\ndef size.big() -> Nat:\n  23n\n\n# LAW: sort permutes its input\nlaw sort_perm:\n  for +x: Nat\n  for +xs: List<&2, Nat>\n  {Sort.count(x, Sort.sort(xs)) == Sort.count(x, xs) : Nat}\n\nlaw bounded:\n  for +n: Nat\n  for e: {n == Nat.add(n, 0n) : Nat}\n  exs m: Nat\n  LE(n, m)\n\nlaw main:\n  IO(Unit)\n\ndef sort_perm(x, xs):\n  match xs:\n    case Nil{}:\n      {==}\n    case h <> t:\n      sort_perm(x, t)\n\ndef greet(name: String) -> IO(Unit):\n  do IO<Unit>:\n    IO.print(\"Hello, \" ++ name)\n    return Unit{}\n\ndef main():\n  greet(\"world\")\n";

fn unit<'a>(file: &'a FileUnits, name: &str) -> &'a Unit {
    file.units.iter().find(|u| u.name == name).unwrap()
}

#[test]
fn bend_defs_types_and_laws_are_units_with_their_roles() {
    let file = parse(Path::new("demos/sort/LAWS.bend"), SORT).unwrap();
    let named: Vec<(&str, Kind, usize)> = file
        .units
        .iter()
        .map(|u| (u.name.as_str(), u.kind, u.line))
        .collect();
    assert_eq!(
        named,
        [
            ("Shape", Kind::Type, 5),
            ("LE", Kind::Function, 10),
            ("area", Kind::Function, 19),
            ("size.big", Kind::Function, 26),
            ("sort_perm", Kind::Law, 30),
            ("bounded", Kind::Law, 35),
            ("main", Kind::Law, 41),
            ("sort_perm", Kind::Function, 44),
            ("greet", Kind::Function, 51),
            ("main", Kind::Function, 56),
        ]
    );
    assert_imports(&file, &["Sort"]);
    // A def that computes a type, a proof of a claim, and code.
    assert_eq!(unit(&file, "LE").role, Role::TypeLevel);
    let proof = file
        .units
        .iter()
        .find(|u| u.name == "sort_perm" && u.kind == Kind::Function)
        .unwrap();
    assert_eq!(proof.role, Role::Proof);
    assert_eq!(unit(&file, "area").role, Role::Code);
    // Effects: a def returning IO, and one that fills an IO law.
    let greet = unit(&file, "greet");
    assert!(greet.effects && greet.joins_text && greet.reaches_out(true));
    let main = file
        .units
        .iter()
        .find(|u| u.name == "main" && u.kind == Kind::Function)
        .unwrap();
    assert!(main.effects);
    assert!(!unit(&file, "area").reaches_out(true) && unit(&file, "area").reaches_out(false));
    assert!(!proof.reaches_out(true) && !unit(&file, "sort_perm").callable());
    // A constant def names its value; other literals stay candidates.
    assert!(unit(&file, "size.big").literals.is_empty());
    let values: Vec<&str> = unit(&file, "area")
        .literals
        .iter()
        .map(|l| l.text.as_str())
        .collect();
    assert_eq!(values, ["3"]);
    assert!(crate::syntax::supported(Path::new("main.bend")));
}

#[test]
fn bend_calls_name_dotted_defs_and_their_unaliased_names() {
    let file = parse(Path::new("demos/sort/LAWS.bend"), SORT).unwrap();
    let law = unit(&file, "sort_perm");
    for call in ["Sort.count", "count", "Sort.sort", "sort"] {
        assert!(law.calls.contains(call), "{call}: {:?}", law.calls);
    }
    assert!(unit(&file, "greet").calls.contains("IO.print"));
}

#[test]
fn bend_laws_tell_claims_apart_and_read_in_words() {
    let file = parse(Path::new("demos/sort/LAWS.bend"), SORT).unwrap();
    let statement = |name: &str| {
        file.units
            .iter()
            .find(|u| u.name == name && u.kind == Kind::Law)
            .and_then(|u| u.statement.clone())
            .unwrap()
    };
    let none = std::collections::BTreeSet::new();
    let propositions: std::collections::BTreeSet<String> = ["LE".to_string()].into();
    let perm = statement("sort_perm");
    assert!(perm.claim(&none) && perm.general());
    assert_eq!(
        perm.reading(&none),
        "for every x: Nat, for every xs: List<&2, Nat>: Sort.count(x, Sort.sort(xs)) == Sort.count(x, xs)."
    );
    let bounded = statement("bounded");
    assert!(bounded.claim(&none), "a witness makes a claim");
    assert_eq!(
        bounded.reading(&propositions),
        "for every n: Nat, assuming n == Nat.add(n, 0n), there is some m: Nat such that LE(n, m) holds."
    );
    let main = statement("main");
    assert!(!main.claim(&propositions) && !main.general(), "a signature");
}

#[test]
fn a_bend_test_is_its_whole_file_and_its_output_is_no_comment() {
    let test = "# array reads wrap around\nimport Base\n\ndef main() -> U32:\n  a = [0 : U32*8n]\n  a[9]\n\n#|0\n#|exit 0\n";
    let path = Path::new("tests/run/array_wrap.bend");
    let located = crate::test_locations::locate_tests(path, test).unwrap();
    assert!(located.whole_file);
    let cases = crate::analysis::test_map::cases(path, test).unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!((cases[0].name.as_str(), cases[0].line), ("array_wrap", 1));
    assert_eq!(cases[0].end_line, 9, "the expected output is the assertion");
    let file = parse(path, test).unwrap();
    let comments = crate::analysis::comments::comments(path, test, &file.units).unwrap();
    let texts: Vec<&str> = comments.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, ["# array reads wrap around"]);
}

#[test]
fn a_bend_test_holding_a_syntax_error_is_left_out_whole_with_its_first_error() {
    // tree-sitter-bend2 lacks typed lets in a def's body.
    let test = "import Base\n\ndef helper(n: U32) -> U32:\n  +x : U32 = n\n  x\n\ndef main() -> U32:\n  helper(1)\n\n#|1\n";
    let path = Path::new("tests/run/typed_let.bend");
    let mut file = parse(path, test).unwrap();
    let kept: Vec<&str> = file.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(kept, ["main"]);
    let broken = crate::analysis::test_map::broken_cases(path, test).unwrap();
    file.leave_out_tests(broken, test);
    assert!(file.units.is_empty(), "its defs sit in the test");
    let left_out: Vec<(&str, usize, usize, usize)> = file
        .left_out
        .iter()
        .map(|l| (l.name.as_str(), l.line, l.end_line, l.error_line))
        .collect();
    assert_eq!(left_out, [("typed_let", 1, 10, 4)]);
}

#[test]
fn bend_1_is_skipped_with_its_own_reason_and_bend_2_imports_link_files() {
    let bend1 = "type MyTree(t):\n  Node { val: t }\n\ndef main() -> u24:\n  return MyTree/Node { val: 1 }\n";
    let error = crate::syntax::parse(Path::new("examples/tree.bend"), bend1).unwrap_err();
    assert_eq!(crate::syntax::skip_reason(&error), crate::syntax::BEND1);
    let imports = crate::analysis::imports::Imports::new(
        Path::new("demos/sort/PROOF.bend"),
        "import Base\nimport ./LAWS.bend as Laws\nimport ../lib/list.bend as L\n",
    );
    assert!(imports.reach(Path::new("demos/sort/LAWS.bend")));
    assert!(imports.reach(Path::new("demos/lib/list.bend")));
    assert!(imports.reach(Path::new("bend2/base.bend")), "Base");
    assert!(!imports.reach(Path::new("demos/sort/main.bend")));
    assert!(!imports.reach(Path::new("demos/sort/LAWS.ts")));
}
