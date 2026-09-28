//! The generic tier: units from each language's tag query.
use super::*;

const KOTLIN: &str = "package shop\n\n// Grades a score.\nfun grade(x: Int): String {\n    if (x > 90) {\n        return \"A\"\n    } else if (x > 80) {\n        return \"B\"\n    } else {\n        for (i in 0..3) {\n            while (true) {\n                println(i)\n            }\n        }\n        return label(x)\n    }\n}\n\nclass Box(val size: Int) : Shape {\n    override fun area(): Double {\n        fun half() = size / 2\n        val side = size.toDouble()\n        return side * side\n    }\n\n    companion object {\n        fun make() = Box(1)\n    }\n}\n\ndata class Point(val x: Int, val y: Int)\n";

const C: &str = "#include <stdio.h>\n\n/* A row of text. */\ntypedef struct erow {\n    int size;\n    char *chars;\n} erow;\n\nstruct point { int x; int y; };\n\nint editorRowCx(erow *row, int cx);\n\n/* Count the tabs before `cx`. */\nstatic int tabs(erow *row, int cx) {\n    int n = 0;\n    for (int j = 0; j < cx; j++) {\n        if (row->chars[j] == '\\t') {\n            n++;\n        } else if (row->chars[j] == ' ') {\n            n += 0;\n        } else {\n            n -= 0;\n        }\n    }\n\n    printf(\"%d\", n);\n    return n;\n}\n\nchar **lines(int count) {\n    char **out = malloc(count);\n    return out;\n}\n";

const CPP: &str = "namespace shop {\n// A cart.\nclass Cart {\n public:\n  int total() const {\n    int sum = 0;\n    for (auto& i : items_) {\n      sum += i;\n    }\n    return sum;\n  }\n  void add(int x);\n};\n\nvoid Cart::add(int x) {\n  if (x > 0) {\n    items_.push_back(x);\n  }\n}\n\ntemplate <typename T>\nT twice(T x) {\n  return helper(x) * 2;\n}\n}  // namespace shop\n";

const SWIFT: &str = "/// Grades a score.\nfunc grade(_ x: Int) -> String {\n    if x > 90 {\n        return \"A\"\n    } else if x > 80 {\n        return \"B\"\n    } else {\n        for i in 0..<3 {\n            print(i)\n        }\n        return label(x)\n    }\n}\n\nprotocol Shape {\n    func area() -> Double\n}\n\nstruct Box: Shape {\n    let size: Double\n\n    init(size: Double) {\n        self.size = size\n    }\n\n    func area() -> Double {\n        let side = size\n        return side * side\n    }\n}\n\nextension Box {\n    func describe() -> String {\n        return \"box\"\n    }\n}\n\nenum Kind { case a, b }\n";

const BASH: &str = "#!/usr/bin/env bash\n# Deploy helper.\nset -euo pipefail\n\n# Build the image.\nbuild() {\n  local tag=\"$1\"\n  if [ -z \"$tag\" ]; then\n    echo \"missing tag\" >&2\n    return 1\n  elif [ \"$tag\" = \"latest\" ]; then\n    echo \"latest\"\n  else\n    docker build -t \"app:$tag\" .\n  fi\n\n  for f in *.txt; do\n    while read -r line; do\n      log \"$line\"\n    done < \"$f\"\n  done\n}\n\nfunction deploy {\n  build \"$1\"\n  kubectl apply -f k8s/\n}\n\ndeploy \"${1:-latest}\"\n";

const DART: &str = "import 'package:flutter/material.dart';\n\n/// A counter.\nclass Counter extends StatelessWidget {\n  final int start;\n  const Counter({super.key, required this.start});\n\n  @override\n  Widget build(BuildContext context) {\n    if (start > 3) {\n      return Text('big');\n    } else if (start > 1) {\n      return Text('mid');\n    }\n    return Text('small $start');\n  }\n\n  int get twice => start * 2;\n}\n\nint top(int x) {\n  return x + 1;\n}\n\nenum Color { red, green }\n";

const SCALA: &str = "package shop\n\n/** A cart. */\ncase class Cart(items: List[Item]) {\n  def total: BigDecimal = items.map(_.price).sum\n\n  def add(item: Item): Cart = {\n    if (item.price > 0) {\n      copy(items = item :: items)\n    } else if (item.price == 0) {\n      this\n    } else {\n      throw new IllegalArgumentException(\"negative\")\n    }\n  }\n}\n\nobject Cart {\n  def empty: Cart = Cart(Nil)\n}\n\ntrait Priced {\n  def price: BigDecimal\n}\n\nenum Color:\n  case Red, Green\n";

const ELIXIR: &str = "defmodule Shop.Cart do\n  @moduledoc \"A cart.\"\n\n  # Adds an item.\n  def add(cart, item) when is_map(item) do\n    if item.price > 0 do\n      Map.update(cart, :items, [item], fn items ->\n        [item | items]\n      end)\n    else\n      cart\n    end\n  end\n\n  def total(cart), do: Enum.sum(cart.items)\n\n  defp helper(x) do\n    x |> normalize() |> String.trim()\n  end\nend\n";

const LUA: &str = "-- Utilities.\nlocal M = {}\n\n--- Adds two numbers.\n-- Returns the larger when they differ.\nfunction M.add(a, b)\n  if a > b then\n    return a\n  elseif a == b then\n    return 0\n  else\n    for i = 1, 10 do\n      print(i)\n    end\n  end\n  return a + b\nend\n\nfunction M:send(x)\n  return self.value + x\nend\n\nlocal function helper(x)\n  return M.add(x, 1)\nend\n\nM.other = function(y)\n  return helper(y)\nend\n\nreturn M\n";

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

/// The function, method and type names a grammar's own `tags.scm` finds in
/// `source`, as GitHub's code navigation shows them: C and C++ tag the
/// declarator of a prototype as they tag a definition's, so a declarator
/// outside every function definition is left out.
fn navigation_names(language: tree_sitter::Language, tags: &str, source: &str) -> Vec<String> {
    use tree_sitter::StreamingIterator;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).unwrap();
    let tree = parser.parse(source, None).unwrap();
    let query = tree_sitter::Query::new(&language, tags).unwrap();
    let captures = query.capture_names();
    let definitions = ["function", "method", "class", "interface", "module", "type"];
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    let mut names = Vec::new();
    while let Some(found) = matches.next() {
        let kind = |c: &tree_sitter::QueryCapture<'_>| captures[c.index as usize];
        let defined = found.captures().iter().find(|c| {
            kind(c)
                .strip_prefix("definition.")
                .is_some_and(|d| definitions.contains(&d))
        });
        let name = found.captures().iter().find(|c| kind(c) == "name");
        let (Some(defined), Some(name)) = (defined, name) else {
            continue;
        };
        let prototype = defined.node.kind() == "function_declarator"
            && std::iter::successors(defined.node.parent(), |n| n.parent())
                .all(|n| n.kind() != "function_definition");
        if !prototype {
            names.push(name.node.utf8_text(source.as_bytes()).unwrap().to_string());
        }
    }
    names
}

#[test]
fn every_definition_github_s_navigation_tags_is_a_unit_or_owns_one() {
    let samples = [
        (
            "editor.c",
            C,
            tree_sitter_c::LANGUAGE,
            tree_sitter_c::TAGS_QUERY,
        ),
        (
            "cart.cpp",
            CPP,
            tree_sitter_cpp::LANGUAGE,
            tree_sitter_cpp::TAGS_QUERY,
        ),
        (
            "Shop.swift",
            SWIFT,
            tree_sitter_swift::LANGUAGE,
            tree_sitter_swift::TAGS_QUERY,
        ),
        (
            "counter.dart",
            DART,
            tree_sitter_dart::LANGUAGE,
            tree_sitter_dart::TAGS_QUERY,
        ),
        (
            "cart.ex",
            ELIXIR,
            tree_sitter_elixir::LANGUAGE,
            tree_sitter_elixir::TAGS_QUERY,
        ),
        (
            "util.lua",
            LUA,
            tree_sitter_lua::LANGUAGE,
            tree_sitter_lua::TAGS_QUERY,
        ),
    ];
    for (path, source, language, tags) in samples {
        let units = parse(Path::new(path), source).unwrap().units;
        let tagged = navigation_names(language.into(), tags, source);
        assert!(!tagged.is_empty(), "{path}");
        for name in tagged {
            assert!(
                units
                    .iter()
                    .any(|u| u.short_name == name || u.owner == name),
                "{path}: {name}"
            );
        }
    }
}
