//! The generic tier: units from each language's tag query.
use super::*;

const KOTLIN: &str = "package shop\n\n// Grades a score.\nfun grade(x: Int): String {\n    if (x > 90) {\n        return \"A\"\n    } else if (x > 80) {\n        return \"B\"\n    } else {\n        for (i in 0..3) {\n            while (true) {\n                println(i)\n            }\n        }\n        return label(x)\n    }\n}\n\nclass Box(val size: Int) : Shape {\n    override fun area(): Double {\n        fun half() = size / 2\n        val side = size.toDouble()\n        return side * side\n    }\n\n    companion object {\n        fun make() = Box(1)\n    }\n}\n\ndata class Point(val x: Int, val y: Int)\n";

pub(super) const C: &str = "#include <stdio.h>\n\n/* A row of text. */\ntypedef struct erow {\n    int size;\n    char *chars;\n} erow;\n\nstruct point { int x; int y; };\n\nint editorRowCx(erow *row, int cx);\n\n/* Count the tabs before `cx`. */\nstatic int tabs(erow *row, int cx) {\n    int n = 0;\n    for (int j = 0; j < cx; j++) {\n        if (row->chars[j] == '\\t') {\n            n++;\n        } else if (row->chars[j] == ' ') {\n            n += 0;\n        } else {\n            n -= 0;\n        }\n    }\n\n    printf(\"%d\", n);\n    return n;\n}\n\nchar **lines(int count) {\n    char **out = malloc(count);\n    return out;\n}\n";

pub(super) const CPP: &str = "namespace shop {\n// A cart.\nclass Cart {\n public:\n  int total() const {\n    int sum = 0;\n    for (auto& i : items_) {\n      sum += i;\n    }\n    return sum;\n  }\n  void add(int x);\n};\n\nvoid Cart::add(int x) {\n  if (x > 0) {\n    items_.push_back(x);\n  }\n}\n\ntemplate <typename T>\nT twice(T x) {\n  return helper(x) * 2;\n}\n}  // namespace shop\n";

pub(super) const SWIFT: &str = "/// Grades a score.\nfunc grade(_ x: Int) -> String {\n    if x > 90 {\n        return \"A\"\n    } else if x > 80 {\n        return \"B\"\n    } else {\n        for i in 0..<3 {\n            print(i)\n        }\n        return label(x)\n    }\n}\n\nprotocol Shape {\n    func area() -> Double\n}\n\nstruct Box: Shape {\n    let size: Double\n\n    init(size: Double) {\n        self.size = size\n    }\n\n    func area() -> Double {\n        let side = size\n        return side * side\n    }\n}\n\nextension Box {\n    func describe() -> String {\n        return \"box\"\n    }\n}\n\nenum Kind { case a, b }\n";

const BASH: &str = "#!/usr/bin/env bash\n# Deploy helper.\nset -euo pipefail\n\n# Build the image.\nbuild() {\n  local tag=\"$1\"\n  if [ -z \"$tag\" ]; then\n    echo \"missing tag\" >&2\n    return 1\n  elif [ \"$tag\" = \"latest\" ]; then\n    echo \"latest\"\n  else\n    docker build -t \"app:$tag\" .\n  fi\n\n  for f in *.txt; do\n    while read -r line; do\n      log \"$line\"\n    done < \"$f\"\n  done\n}\n\nfunction deploy {\n  build \"$1\"\n  kubectl apply -f k8s/\n}\n\ndeploy \"${1:-latest}\"\n";

pub(super) const DART: &str = "import 'package:flutter/material.dart';\n\n/// A counter.\nclass Counter extends StatelessWidget {\n  final int start;\n  const Counter({super.key, required this.start});\n\n  @override\n  Widget build(BuildContext context) {\n    if (start > 3) {\n      return Text('big');\n    } else if (start > 1) {\n      return Text('mid');\n    }\n    return Text('small $start');\n  }\n\n  int get twice => start * 2;\n}\n\nint top(int x) {\n  return x + 1;\n}\n\nenum Color { red, green }\n";

const SCALA: &str = "package shop\n\n/** A cart. */\ncase class Cart(items: List[Item]) {\n  def total: BigDecimal = items.map(_.price).sum\n\n  def add(item: Item): Cart = {\n    if (item.price > 0) {\n      copy(items = item :: items)\n    } else if (item.price == 0) {\n      this\n    } else {\n      throw new IllegalArgumentException(\"negative\")\n    }\n  }\n}\n\nobject Cart {\n  def empty: Cart = Cart(Nil)\n}\n\ntrait Priced {\n  def price: BigDecimal\n}\n\nenum Color:\n  case Red, Green\n";

pub(super) const ELIXIR: &str = "defmodule Shop.Cart do\n  @moduledoc \"A cart.\"\n\n  # Adds an item.\n  def add(cart, item) when is_map(item) do\n    if item.price > 0 do\n      Map.update(cart, :items, [item], fn items ->\n        [item | items]\n      end)\n    else\n      cart\n    end\n  end\n\n  def total(cart), do: Enum.sum(cart.items)\n\n  defp helper(x) do\n    x |> normalize() |> String.trim()\n  end\nend\n";

pub(super) const LUA: &str = "-- Utilities.\nlocal M = {}\n\n--- Adds two numbers.\n-- Returns the larger when they differ.\nfunction M.add(a, b)\n  if a > b then\n    return a\n  elseif a == b then\n    return 0\n  else\n    for i = 1, 10 do\n      print(i)\n    end\n  end\n  return a + b\nend\n\nfunction M:send(x)\n  return self.value + x\nend\n\nlocal function helper(x)\n  return M.add(x, 1)\nend\n\nM.other = function(y)\n  return helper(y)\nend\n\nreturn M\n";

/// Each unit's name, kind and the line its definition starts on, below
/// the comments its span includes.
fn outline(path: &str, source: &str) -> Vec<(String, Kind, usize)> {
    let file = parse(Path::new(path), source).unwrap();
    assert!(file.parsed && file.generic, "{path}");
    file.units
        .into_iter()
        .map(|u| (u.name, u.kind, u.line))
        .collect()
}

/// A unit's body lines, control flow (nesting, then its branch chain), the
/// number of blocks a split could extract, and the names it calls.
fn facts(path: &str, source: &str, name: &str) -> (usize, (usize, usize), usize, Vec<String>) {
    let file = parse(Path::new(path), source).unwrap();
    let unit = file.units.iter().find(|u| u.name == name).unwrap();
    (
        unit.body_lines,
        (unit.nesting, unit.branch_chain),
        unit.blocks.len(),
        unit.calls.iter().cloned().collect(),
    )
}

fn owned(names: &[(&str, Kind, usize)]) -> Vec<(String, Kind, usize)> {
    names
        .iter()
        .map(|(name, kind, line)| (name.to_string(), *kind, *line))
        .collect()
}

#[test]
fn kotlin_functions_classes_and_objects_are_units() {
    assert_eq!(
        outline("Shop.kt", KOTLIN),
        owned(&[
            ("grade", Kind::Function, 4),
            ("Box::area", Kind::Method, 20),
            ("Box::make", Kind::Method, 27),
            ("Point", Kind::Type, 31),
        ])
    );
    // `else if` continues the chain; a local function is part of its
    // enclosing one; a companion object's function is its class's.
    assert_eq!(
        facts("Shop.kt", KOTLIN, "grade"),
        (9, (3, 3), 0, vec!["label".into(), "println".into()])
    );
    assert_eq!(facts("Shop.kt", KOTLIN, "Box::make").0, 1);
}

#[test]
fn c_functions_and_types_are_units_and_prototypes_are_not() {
    assert_eq!(
        outline("editor.c", C),
        owned(&[
            ("erow", Kind::Type, 4),
            ("point", Kind::Type, 9),
            ("tabs", Kind::Function, 14),
            ("lines", Kind::Function, 30),
        ])
    );
    assert_eq!(
        facts("editor.c", C, "tabs"),
        (10, (2, 3), 2, vec!["printf".into()])
    );
    let cpp = outline("cart.cpp", CPP);
    assert_eq!(
        cpp,
        owned(&[
            ("Cart::total", Kind::Method, 5),
            ("Cart::add", Kind::Method, 15),
            ("twice", Kind::Function, 21),
        ])
    );
    let file = parse(Path::new("cart.cpp"), CPP).unwrap();
    assert!(file.units[2].signature.starts_with("template <typename T>"));
}

#[test]
fn swift_types_extensions_protocols_and_initializers_are_units() {
    assert_eq!(
        outline("Shop.swift", SWIFT),
        owned(&[
            ("grade", Kind::Function, 2),
            ("Shape::area", Kind::Method, 16),
            ("Box::init", Kind::Method, 22),
            ("Box::area", Kind::Method, 26),
            ("Box::describe", Kind::Method, 33),
            ("Kind", Kind::Type, 38),
        ])
    );
    assert_eq!(
        facts("Shop.swift", SWIFT, "grade"),
        (8, (2, 3), 0, vec!["label".into(), "print".into()])
    );
}

#[test]
fn bash_functions_are_units_and_commands_their_calls() {
    assert_eq!(
        outline("deploy.sh", BASH),
        owned(&[("build", Kind::Function, 6), ("deploy", Kind::Function, 24)])
    );
    let (lines, flow, blocks, calls) = facts("deploy.sh", BASH, "build");
    assert_eq!((lines, flow, blocks), (14, (2, 3), 2));
    assert!(calls.contains(&"docker".to_string()) && calls.contains(&"log".to_string()));
}

#[test]
fn dart_scala_elixir_and_lua_definitions_are_units() {
    assert_eq!(
        outline("counter.dart", DART),
        owned(&[
            ("Counter::build", Kind::Method, 8),
            ("Counter::twice", Kind::Method, 18),
            ("top", Kind::Function, 21),
            ("Color", Kind::Type, 25),
        ])
    );
    assert_eq!(facts("counter.dart", DART, "Counter::build").1, (1, 2));
    assert_eq!(
        outline("Cart.scala", SCALA),
        owned(&[
            ("Cart::total", Kind::Method, 5),
            ("Cart::add", Kind::Method, 7),
            ("Cart::empty", Kind::Method, 19),
            ("Priced::price", Kind::Method, 23),
            ("Color", Kind::Type, 26),
        ])
    );
    assert_eq!(facts("Cart.scala", SCALA, "Cart::add").1, (1, 3));
    assert_eq!(
        outline("cart.ex", ELIXIR),
        owned(&[
            ("Shop.Cart::add", Kind::Method, 5),
            ("Shop.Cart::total", Kind::Method, 15),
            ("Shop.Cart::helper", Kind::Method, 17),
        ])
    );
    // Its `do` blocks and `fn`s nest; the function head is no call.
    let (lines, flow, _, calls) = facts("cart.ex", ELIXIR, "Shop.Cart::add");
    assert_eq!((lines, flow), (9, (2, 0)));
    assert!(calls.contains(&"update".to_string()) && !calls.contains(&"add".to_string()));
    assert_eq!(
        outline("util.lua", LUA),
        owned(&[
            ("M::add", Kind::Method, 6),
            ("M::send", Kind::Method, 19),
            ("helper", Kind::Function, 23),
            ("M::other", Kind::Method, 27),
        ])
    );
    let file = parse(Path::new("util.lua"), LUA).unwrap();
    assert_eq!(file.units[0].doc, "Adds two numbers.");
    assert_eq!(facts("util.lua", LUA, "M::add").1, (2, 3));
}

#[test]
fn a_generic_file_of_nothing_but_an_error_leaves_no_unit_and_says_so() {
    for (path, source) in [
        ("broken.kt", "fun broken( {"),
        ("broken.c", "int broken( {"),
        ("broken.swift", "func broken( {"),
    ] {
        let file = parse(Path::new(path), source).unwrap();
        assert!(file.units.is_empty() && file.partial(), "{path}");
    }
}

/// Three functions in Kotlin, C or Swift, the middle one holding a comment
/// and, on the line below it, `statement`.
fn three(path: &str, statement: &str) -> String {
    let function = |name: &str, body: &str| match path.rsplit('.').next() {
        Some("kt") => format!("fun {name}(x: Int): Int {{\n{body}}}\n"),
        Some("c") => format!("int {name}(int x) {{\n{body}}}\n"),
        _ => format!("func {name}(_ x: Int) -> Int {{\n{body}}}\n"),
    };
    let end = if path.ends_with(".c") { ";" } else { "" };
    format!(
        "{}\n{}\n{}",
        function("first", &format!("    return x + 1{end}\n")),
        function(
            "broken",
            &format!("    // Doubles it.\n    {statement}\n    return y{end}\n")
        ),
        function("last", &format!("    return x - 1{end}\n"))
    )
}

#[test]
fn a_generic_definition_holding_a_syntax_error_is_left_out_by_name() {
    for (path, statement) in [
        ("math.kt", "val y: = x"),
        ("math.c", "int y = (x + 2;"),
        ("math.swift", "let y = x + * 2"),
    ] {
        let source = three(path, statement);
        let file = parse(Path::new(path), &source).unwrap();
        let kept: Vec<&str> = file.units.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(kept, ["first", "last"], "{path}");
        let left_out: Vec<(&str, usize, usize, usize)> = file
            .left_out
            .iter()
            .map(|l| (l.name.as_str(), l.line, l.end_line, l.error_line))
            .collect();
        assert_eq!(left_out, [("broken", 5, 9, 7)], "{path}");
        // The comment inside it goes with it; the other functions are whole.
        let comment = source.find("Doubles").unwrap();
        assert!(!file.intact(&(comment..comment + 7)), "{path}");
        assert!(file.units.iter().all(|u| file.intact(&u.span)), "{path}");
    }
}

#[test]
fn definitions_the_parser_could_not_read_are_left_out_by_their_lines() {
    // tree-sitter-kotlin-ng reads `broken` and `last` as one error.
    let source = three("math.kt", "val = x * 2");
    let file = parse(Path::new("math.kt"), &source).unwrap();
    let kept: Vec<&str> = file.units.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(kept, ["first"]);
    let code: Vec<(String, usize, usize)> = file
        .left_out_code(&source)
        .into_iter()
        .map(|l| (l.name, l.line, l.end_line))
        .collect();
    assert_eq!(code, [(String::new(), 5, 13)]);
}
