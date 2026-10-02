//! Bend 2, the language of bendlang/bend (2.0.x): pure, affine and
//! dependently typed, with Python-shaped `def`s, `type`s and `law`s. A law
//! states a claim that a `def` of the same name proves; Base also declares
//! its primitives and opaque handles as laws without one. A test is a
//! program whose file ends in the `#|` lines its run must print.
//!
//! Bend 1, HigherOrderCO's 2024 language, shares the `.bend` extension and
//! not the syntax, so its files are told apart before parsing and skipped.
use super::text;
use std::path::Path;
use tree_sitter::Node;

mod bend1;
mod defs;
mod statement;

pub(crate) use bend1::bend1;
pub(crate) use defs::{effectful, joins_text, proof, type_level};
pub use statement::Statement;
pub(crate) use statement::statement;

/// The language's name in reports and in what Jev is asked, so a model
/// that knows Bend 1 does not read Bend 2 code as it.
pub(crate) const LANGUAGE: &str = "Bend 2";

pub(crate) fn file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "bend")
}

/// A project's `LAWS.bend` or `PROOF.bend`: by Bend 2's convention its laws
/// and their proofs live in these two files at its root, and `bend
/// PROOF.bend` checks them all, so they are not split into modules. Five
/// of 32 file-organization findings on twelve Bend 2 projects asked to.
pub(crate) fn law_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name == "LAWS.bend" || name == "PROOF.bend")
}

/// A file of proofs: a `PROOF.bend`, a file named after what it proves
/// (`padding_proof.bend`, mylsm's `BloomSafeProof.bend`) or one under a
/// `proof` or `proofs` directory, as bend-collections keeps its lemmas.
/// Its defs are steps of proofs, and the comments of its laws say how a
/// proof goes: on the 16 projects where such files were first judged, 12
/// of 14 function-simplification findings in them were wrong and the rest
/// debatable, as were 14 of 15 law findings, and 36 of 40 hardcoded-value
/// findings were wrong.
pub(crate) fn proof_file(path: &Path) -> bool {
    let named = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|stem| stem.ends_with("proof") || stem.ends_with("proofs"));
    let placed = path.parent().is_some_and(|dir| {
        dir.iter().any(|part| {
            let part = part.to_string_lossy().to_ascii_lowercase();
            part == "proof" || part == "proofs"
        })
    });
    file(path) && (named || placed)
}

/// Whether a Bend 2 file defines `main`, the def its run starts from.
pub(crate) fn defines_main(source: &str) -> bool {
    source.lines().any(|line| {
        line.strip_prefix("def main")
            .is_some_and(|rest| rest.starts_with(['(', ':', ' ']))
    })
}

/// The byte where a test's expected output starts: the `#|` lines that end
/// the file, which its run must print. Blank lines may follow them.
pub(crate) fn expected_output(source: &str) -> Option<usize> {
    let body = source.trim_end();
    let mut start = None;
    let mut end = body.len();
    loop {
        let line_start = body[..end].rfind('\n').map_or(0, |newline| newline + 1);
        if !body[line_start..end].trim_start().starts_with("#|") {
            return start;
        }
        start = Some(line_start);
        if line_start == 0 {
            return start;
        }
        end = line_start - 1;
    }
}

/// A comment line of a test's expected output, which is no prose to judge.
pub(crate) fn output_line(comment: &str) -> bool {
    comment.starts_with("#|")
}

/// The alias of each module a file imports: `Sort` of `import ./main.bend as Sort`.
pub(crate) fn aliases(root: Node<'_>, source: &str) -> Vec<String> {
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .filter(|c| c.kind() == "import_declaration")
        .filter_map(|c| c.child_by_field_name("alias"))
        .map(|alias| text(alias, source).to_string())
        .collect()
}

/// The file an import names, relative to the importing file: `./main.bend`
/// or `../lib/list.bend`. Base, hub packages (`0x…/main.bend`,
/// `name@version/main.bend`) and other paths name no file of the project.
pub(crate) fn imported_file(importer: &Path, path: &str) -> Option<std::path::PathBuf> {
    if !(path.starts_with("./") || path.starts_with("../")) {
        return None;
    }
    let mut resolved = importer.parent()?.to_path_buf();
    for part in path.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                if !resolved.pop() {
                    return None;
                }
            }
            part => resolved.push(part),
        }
    }
    Some(resolved)
}

/// The import paths of a file's source, read from its lines.
pub(crate) fn import_paths(source: &str) -> impl Iterator<Item = &str> {
    source.lines().filter_map(|line| {
        let rest = line.strip_prefix("import ")?.trim();
        let path = rest.split_whitespace().next()?;
        Some(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bend_1_is_told_apart_from_bend_2() {
        let bend2 = "# LAW: sorted\nimport Base\nimport ./main.bend as Sort\n\ntype Shape is Data:\n  Circle{r: U32}\n\ndef Pong.inside(\n  +x0: U32, +y0: U32\n) -> Bool:\n  ((x0 <= y0) : U32)\n\n@unsafe def loop(n: Nat) -> Nat:\n  loop(n)\n\nlaw sort_sorted:\n  for +xs: List<&2, Nat>\n  Sort.Sorted(Sort.sort(xs))\n\ndef main() -> IO(Unit):\n  do IO<Unit>:\n    x : U32 = (8 / 2 : U32)\n    return Unit{}\n\n#|4\n";
        assert!(!bend1(bend2));
        for bend1_source in [
            "type MyTree(t):\n  Node { val: t }\n  Leaf\n\ndef main() -> u24:\n  return 1\n",
            "def sum(tree):\n  fold tree:\n    case MyTree/Node:\n      return tree.val\n",
            "main = (Sum (Gen 4))\n",
            "(Main) = λx x\n",
            "import lib/import_entry\n\ndef main():\n  return *\n",
            "def main:\n  y = 89\n",
            "data Tree = (Leaf a) | (Both a b)\n",
            "from lib import myFun\n",
            "def gen(d):\n  bend height=0:\n    when height < d:\n      x = 1\n",
            "def main():\n  with IO:\n    * <- IO/print(\"hi\")\n",
        ] {
            assert!(bend1(bend1_source), "{bend1_source}");
        }
        // Division needs spaces in Bend 2; a path or a comment is no name.
        assert!(!bend1("def half(x: U32) -> U32:\n  (x / 2 : U32) # a/b\n"));
        assert!(!bend1(
            "def e(x: U32) -> IO(U32):\n  import \"./e/f.c\"\n  import \"./e.js\"\n"
        ));
    }

    #[test]
    fn a_test_ends_in_its_expected_output() {
        let test = "import Base\n\ndef main() -> U32:\n  7\n\n#|7\n#|exit 0\n\n";
        let start = expected_output(test).unwrap();
        assert_eq!(&test[start..], "#|7\n#|exit 0\n\n");
        assert_eq!(expected_output("def main() -> U32:\n  7\n# 7\n"), None);
        assert_eq!(expected_output("#|only\n"), Some(0));
    }

    #[test]
    fn files_of_proofs_are_told_by_their_name_or_directory() {
        for path in [
            "PROOF.bend",
            "demos/sort/PROOF.bend",
            "padding_proof.bend",
            "proofs/BloomSafeProof.bend",
            "proofs/lib/lemmas/map.bend",
            "src/proof/nat.bend",
        ] {
            assert!(proof_file(Path::new(path)), "{path}");
        }
        for path in [
            "LAWS.bend",
            "src/proofreader.bend",
            "spec/containers/lru.bend",
            "proofs/notes.md",
        ] {
            assert!(!proof_file(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn imports_resolve_relative_to_the_importer() {
        let importer = Path::new("demos/sort/PROOF.bend");
        assert_eq!(
            imported_file(importer, "./LAWS.bend").unwrap(),
            Path::new("demos/sort/LAWS.bend")
        );
        assert_eq!(
            imported_file(importer, "../lib/list.bend").unwrap(),
            Path::new("demos/lib/list.bend")
        );
        assert_eq!(imported_file(importer, "Base"), None);
        assert_eq!(imported_file(importer, "0xabc/main.bend"), None);
        let paths: Vec<&str> =
            import_paths("import Base\nimport ./main.bend as M\n  import \"./e.c\"\n").collect();
        assert_eq!(paths, ["Base", "./main.bend"]);
    }
}
