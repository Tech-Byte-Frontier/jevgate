//! Bend 1, HigherOrderCO's 2024 language, shares the `.bend` extension and
//! not the syntax: its files are told apart before parsing and skipped.

/// Whether `.bend` source is Bend 1: it has no line only Bend 2 writes, and
/// a top-level line only Bend 1 writes (`main = …`, `(Sum Leaf) = 0`,
/// `data`, `object`, `from lib import f`, `import lib/a` without an alias,
/// `def main:` without parentheses, `type Tree:` without `is`), a
/// Bend 1 statement such as `fold t:` or `bend x = 0:`, or a name with a
/// slash, such as `List/Cons`. Bend 2 opens its top-level lines only with
/// `def`, `law`, `type … is`, `import`, `@`, a comment or the `)` closing a
/// long parameter list, spells its names with dots, and has none of those
/// statements; its own tests of refused syntax (`type Kk:`) import Base and
/// state laws. Checked before parsing: Bend 2's grammar took over ten
/// minutes on a Bend 1 test of 1 MB.
pub(crate) fn bend1(source: &str) -> bool {
    !bend2_only(source)
        && source.lines().any(|line| {
            let code = line.trim_end();
            let trimmed = code.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return false;
            }
            let top_level = trimmed.len() == code.len();
            (top_level && bend1_declaration(trimmed))
                || bend1_statement(trimmed)
                || slash_name(trimmed)
        })
}

/// A line only Bend 2 writes: `import Base`, a law, a datatype's kind, a
/// test's expected output, reflexivity or a `do IO<…>` block.
fn bend2_only(source: &str) -> bool {
    source.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "import Base"
            || line.starts_with("law ")
            || trimmed.starts_with("#|")
            || [" is Data", " is Type", " is Kind(", "{==}", "do IO<"]
                .iter()
                .any(|marker| trimmed.contains(marker))
    })
}

fn bend1_declaration(line: &str) -> bool {
    let first = line
        .split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '@'))
        .next()
        .unwrap_or("");
    match first {
        "law" => false,
        // Bend 2 imports a file with an alias (`import ./x.bend as X`), and
        // every def of it takes parentheses.
        "import" => line != "import Base" && !line.contains(" as "),
        "def" => !line.contains('('),
        // A long Bend 2 header names its kind on a later line.
        "type" => line.ends_with(':') && !line.contains(" is "),
        "data" | "object" | "from" | "hvm" => true,
        _ if line.starts_with('@') || line.starts_with(')') => false,
        // `(Main) = …`, `main = …` and `Foo a b = 0` define by equations.
        _ => line.starts_with('(') || !first.is_empty() && line.contains('='),
    }
}

/// A Bend 1 statement that opens a block: `bend x = 0:`, `fold t:`,
/// `switch n:`, `when c:`, `with IO:`, `if c:`, `elif c:` or `else:`.
fn bend1_statement(line: &str) -> bool {
    const OPENERS: [&str; 7] = [
        "bend ", "fold ", "switch ", "when ", "with ", "if ", "elif ",
    ];
    line.ends_with(':') && (line == "else:" || OPENERS.iter().any(|o| line.starts_with(o)))
}

/// A slash inside a name, as Bend 1 spells `List/Cons` and `IO/print`,
/// outside strings: Bend 2 divides only with spaces around the slash.
fn slash_name(line: &str) -> bool {
    if line.starts_with("import ") {
        return false;
    }
    let mut quoted = false;
    let bytes = line.as_bytes();
    for (at, &byte) in bytes.iter().enumerate() {
        match byte {
            b'"' => quoted = !quoted,
            b'#' if !quoted => return false,
            b'/' if !quoted && at > 0 => {
                let name = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
                if name(bytes[at - 1]) && bytes.get(at + 1).is_some_and(|&b| name(b)) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}
