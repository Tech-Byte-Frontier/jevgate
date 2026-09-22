use anyhow::{Result, ensure};
use std::{cell::RefCell, collections::BTreeMap, path::Path};
use tree_sitter::{Node, Parser, Tree};

const CACHE_ENTRIES: usize = 256;
const CACHE_SOURCE_BYTES: usize = 4 * 1024 * 1024;

struct Parsed {
    source: String,
    tree: Tree,
    used: u64,
}

#[derive(Default)]
struct ParseCache {
    entries: BTreeMap<(String, String), Parsed>,
    bytes: usize,
    clock: u64,
}

impl ParseCache {
    fn get(&mut self, key: &(String, String), source: &str) -> Option<Tree> {
        let parsed = self.entries.get_mut(key)?;
        // Verify exact bytes as well as the digest. This cache knows nothing
        // about filesystem freshness or whether code should be reviewed.
        if parsed.source != source {
            return None;
        }
        self.clock = self.clock.saturating_add(1);
        parsed.used = self.clock;
        Some(parsed.tree.clone())
    }

    fn insert(&mut self, key: (String, String), source: &str, tree: &Tree) {
        if source.len() > CACHE_SOURCE_BYTES {
            return;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes -= previous.source.len();
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + source.len() > CACHE_SOURCE_BYTES
        {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.used)
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else {
                break;
            };
            self.bytes -= self.entries.remove(&oldest).unwrap().source.len();
        }
        self.clock = self.clock.saturating_add(1);
        self.bytes += source.len();
        self.entries.insert(
            key,
            Parsed {
                source: source.into(),
                tree: tree.clone(),
                used: self.clock,
            },
        );
    }
}

thread_local! {
    // Trees are cheap copy-on-write clones. Bound retained source and entries;
    // eviction merely reparses on the next use and never excludes any review.
    static PARSES: RefCell<ParseCache> = RefCell::new(ParseCache::default());
}

pub(crate) fn parse(path: &Path, source: &str) -> Result<Option<Tree>> {
    let extension = path.extension().and_then(|x| x.to_str()).unwrap_or("");
    let language = match extension {
        "rs" => tree_sitter_rust::LANGUAGE,
        "py" => tree_sitter_python::LANGUAGE,
        "js" | "jsx" | "mjs" | "cjs" => tree_sitter_javascript::LANGUAGE,
        "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX,
        _ => return Ok(None),
    };
    let key = (extension.to_owned(), crate::schema::hash(source.as_bytes()));
    if let Some(tree) = PARSES.with(|cache| cache.borrow_mut().get(&key, source)) {
        return Ok(Some(tree));
    }
    let mut parser = Parser::new();
    parser.set_language(&language.into())?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow::anyhow!("Parser did not produce a tree"))?;
    ensure!(
        !tree.root_node().has_error(),
        "Syntax errors: semantic evaluation was not attempted"
    );
    PARSES.with(|cache| cache.borrow_mut().insert(key, source, &tree));
    Ok(Some(tree))
}

fn visit(node: Node<'_>, source: &str, locations: &mut Vec<(String, usize)>) {
    if matches!(
        node.kind(),
        "function_item"
            | "function_definition"
            | "function_declaration"
            | "method_definition"
            | "arrow_function"
            | "function_expression"
    ) {
        let name = node
            .child_by_field_name("name")
            .or_else(|| node.parent().and_then(|p| p.child_by_field_name("name")))
            .and_then(|n| n.utf8_text(source.as_bytes()).ok())
            .unwrap_or("anonymous");
        locations.push((name.to_owned(), node.start_position().row + 1));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, source, locations);
    }
}

/// Exact callable starts, without the display-window fallback used for text files.
pub(crate) fn callable_lines(path: &Path, source: &str) -> Result<Vec<usize>> {
    let mut locations = Vec::new();
    if let Some(tree) = parse(path, source)? {
        visit(tree.root_node(), source, &mut locations);
    }
    Ok(locations.into_iter().map(|(_, line)| line).collect())
}

pub fn collect(path: &Path, source: &str, _root: &Path) -> Result<(bool, Vec<(String, usize)>)> {
    let tree = parse(path, source)?;
    if let Some(tree) = &tree {
        let mut locations = Vec::new();
        visit(tree.root_node(), source, &mut locations);
        if locations.len() <= 256 {
            return Ok((true, locations));
        }
    }
    let lines = source.lines().count().max(1);
    let step = lines.div_ceil(128).max(80);
    Ok((
        tree.is_some(),
        (1..=lines)
            .step_by(step)
            .map(|start| {
                (
                    format!("Lines {start}-{}", (start + step - 1).min(lines)),
                    start,
                )
            })
            .collect(),
    ))
}

fn tokens(node: Node<'_>, source: &str, out: &mut String) {
    if node.kind().contains("comment") {
        return;
    }
    if node.child_count() == 0 {
        if node.kind().contains("identifier") {
            out.push_str("IDENT");
        } else {
            out.push_str(node.utf8_text(source.as_bytes()).unwrap_or_default());
        }
        out.push('\0');
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            tokens(child, source, out);
        }
    }
}

fn token_text(path: &Path, source: &str) -> Option<String> {
    let tree = parse(path, source).ok().flatten()?;
    let mut normalized = String::new();
    tokens(tree.root_node(), source, &mut normalized);
    Some(normalized)
}

pub fn identity(path: &Path, source: &str) -> String {
    let text = token_text(path, source).unwrap_or_else(|| source.to_string());
    crate::schema::hash(text.as_bytes())
}

pub fn semantic_size(path: &Path, source: &str) -> usize {
    token_text(path, source)
        .map(|text| text.bytes().filter(|byte| *byte == 0).count())
        .unwrap_or_else(|| source.split_whitespace().count())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::{InputEdit, Point};

    #[test]
    fn cached_trees_follow_exact_source_and_grammar() {
        let path = Path::new("example.ts");
        let source = "function before() { return 1; }";
        let original = parse(path, source).unwrap().unwrap();
        let repeated = parse(Path::new("other/example.ts"), source)
            .unwrap()
            .unwrap();
        assert_eq!(
            original.root_node().to_sexp(),
            repeated.root_node().to_sexp()
        );
        let changed = "\nfunction after() { return 2; }";
        assert_eq!(
            collect(path, changed, Path::new(".")).unwrap().1,
            vec![("after".into(), 2)]
        );
        assert!(parse(path, "function before( {").is_err());
        assert_eq!(
            collect(path, source, Path::new(".")).unwrap().1,
            vec![("before".into(), 1)]
        );
        let jsx = "const view = <div/>;";
        assert!(parse(Path::new("view.tsx"), jsx).unwrap().is_some());
        assert!(parse(Path::new("view.ts"), jsx).is_err());
        assert!(parse(Path::new("view.txt"), jsx).unwrap().is_none());
    }

    #[test]
    fn editing_a_returned_tree_does_not_change_the_cached_tree() {
        let path = Path::new("example.rs");
        let source = "fn original() {}";
        let mut edited = parse(path, source).unwrap().unwrap();
        edited.edit(&InputEdit {
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: 1,
            start_position: Point::new(0, 0),
            old_end_position: Point::new(0, 0),
            new_end_position: Point::new(1, 0),
        });
        assert!(edited.root_node().has_changes());
        let original = parse(path, source).unwrap().unwrap();
        assert!(!original.root_node().has_changes());
        assert_eq!(original.root_node().end_byte(), source.len());
        assert_eq!(
            collect(path, source, Path::new(".")).unwrap().1,
            vec![("original".into(), 1)]
        );
    }

    #[test]
    fn eviction_and_digest_collision_never_return_the_wrong_source() {
        let source = "fn original() {}";
        let tree = parse(Path::new("example.rs"), source).unwrap().unwrap();
        let key = ("rs".into(), "forced-collision".into());
        let mut cache = ParseCache::default();
        cache.insert(key.clone(), source, &tree);
        assert!(cache.get(&key, "fn changed_() {}").is_none());
        assert!(cache.get(&key, source).is_some());
        for i in 0..CACHE_ENTRIES {
            cache.insert(("rs".into(), i.to_string()), source, &tree);
        }
        assert_eq!(cache.entries.len(), CACHE_ENTRIES);
        assert!(cache.get(&key, source).is_none());
        cache.insert(key.clone(), source, &tree);
        assert_eq!(
            cache.get(&key, source).unwrap().root_node().to_sexp(),
            tree.root_node().to_sexp()
        );
        // Source larger than the retention budget remains parseable; only its
        // optional cache entry is omitted.
        let oversized = format!("// {}\n{source}", "x".repeat(CACHE_SOURCE_BYTES));
        let large = parse(Path::new("large.rs"), &oversized).unwrap().unwrap();
        cache.insert(("rs".into(), "large".into()), &oversized, &large);
        assert!(
            cache
                .get(&("rs".into(), "large".into()), &oversized)
                .is_none()
        );
        assert!(cache.bytes <= CACHE_SOURCE_BYTES);
    }
}
