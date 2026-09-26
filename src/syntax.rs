//! Tree-sitter parsing for the supported languages, with a bounded cache of
//! trees keyed by grammar and exact source.
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

    /// Drop least recently used entries until one more of `bytes` fits.
    fn evict_for(&mut self, bytes: usize) {
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + bytes > CACHE_SOURCE_BYTES {
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
    }

    fn insert(&mut self, key: (String, String), source: &str, tree: &Tree) {
        if source.len() > CACHE_SOURCE_BYTES {
            return;
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes -= previous.source.len();
        }
        self.evict_for(source.len());
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

fn grammar(path: &Path) -> Option<tree_sitter::Language> {
    let language = match extension(path) {
        "rs" => tree_sitter_rust::LANGUAGE,
        "py" => tree_sitter_python::LANGUAGE,
        "js" | "jsx" | "mjs" | "cjs" => tree_sitter_javascript::LANGUAGE,
        "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX,
        "go" => tree_sitter_go::LANGUAGE,
        "cs" => tree_sitter_c_sharp::LANGUAGE,
        "rb" => tree_sitter_ruby::LANGUAGE,
        "php" | "phtml" => tree_sitter_php::LANGUAGE_PHP,
        "java" => tree_sitter_java::LANGUAGE,
        _ => return None,
    };
    Some(language.into())
}

fn extension(path: &Path) -> &str {
    path.extension().and_then(|x| x.to_str()).unwrap_or("")
}

/// Whether a parser supports this file's language.
pub(crate) fn supported(path: &Path) -> bool {
    grammar(path).is_some()
        || crate::components::FORMATS.contains(&extension(path))
        || crate::components::server_template(path)
}

/// The grammar a file is parsed with, and the text parsed in place of its
/// source when that is not its code as written: a component's or server
/// template's scripts, or a project template without its Jinja tags.
fn parsed_text(path: &Path, source: &str) -> Option<(tree_sitter::Language, Option<String>)> {
    let extension = extension(path);
    if crate::components::FORMATS.contains(&extension) {
        let (scripts, language) = crate::components::scripts(extension, source);
        Some((language, Some(scripts)))
    } else if crate::components::server_template(path) {
        let (scripts, language) = crate::components::scripts("html", source);
        Some((language, Some(without_tags(&scripts, true))))
    } else {
        let language = grammar(path)?;
        Some((
            language,
            project_template(path).then(|| without_jinja(source)),
        ))
    }
}

pub(crate) fn parse(path: &Path, source: &str) -> Result<Option<Tree>> {
    let extension = extension(path);
    let server_template = crate::components::server_template(path);
    let Some((language, scripts)) = parsed_text(path, source) else {
        return Ok(None);
    };
    // A template's tree is of its code without the Jinja tags, apart from
    // the same text's tree elsewhere.
    let kind = if scripts.is_some() && grammar(path).is_some() {
        format!("{extension}+jinja")
    } else {
        extension.to_owned()
    };
    let key = (kind, crate::schema::hash(source.as_bytes()));
    let tree = match PARSES.with(|cache| cache.borrow_mut().get(&key, source)) {
        Some(tree) => tree,
        None => {
            let mut parser = Parser::new();
            parser.set_language(&language)?;
            let tree = parser
                .parse(scripts.as_deref().unwrap_or(source), None)
                .ok_or_else(|| anyhow::anyhow!("Parser did not produce a tree"))?;
            PARSES.with(|cache| cache.borrow_mut().insert(key, source, &tree));
            tree
        }
    };
    // Whether errors are tolerable depends on the path, not only the source.
    ensure!(
        if template(path, source) && !server_template {
            !tree.root_node().has_error()
        } else {
            tolerable(tree.root_node(), source.len())
        },
        "Syntax errors: semantic evaluation was not attempted"
    );
    Ok(Some(tree))
}

/// Error regions a file may hold and still be judged, and the part of its
/// source they may cover in all: one byte in eight.
const ERROR_REGIONS: usize = 3;
const ERROR_SHARE: usize = 8;

/// A generator template, whose placeholders are no syntax of its language:
/// a file under a `templates` directory, or one holding ERB tags (`<%=`)
/// or `dotnet new` conditions (`//#if`). Its parse errors keep it unjudged.
fn template(path: &Path, source: &str) -> bool {
    path.iter()
        .any(|part| matches!(part.to_str(), Some("templates" | "template")))
        || source.contains("<%")
        || source.contains("//#if")
}

/// A file of a project template such as a cookiecutter's, under a directory
/// whose name holds a `{{ … }}` placeholder: its Jinja tags are no syntax of
/// its language, and 31 of cookiecutter-django's Python and JavaScript
/// files, the generated application's settings, models, views and tests,
/// were skipped for syntax errors.
fn project_template(path: &Path) -> bool {
    path.iter().any(|part| {
        part.to_str()
            .is_some_and(|p| p.contains("{{") && p.contains("}}"))
    })
}

/// The source with its Jinja statements and comments blanked and each
/// `{{ … }}` placeholder turned into an identifier of the same length, so
/// that byte offsets and lines stay the file's: `from {{ slug }}.users
/// import User` reads as an import, and both branches of an `{% if %}` stay.
fn without_jinja(source: &str) -> String {
    without_tags(source, false)
}

/// Jinja's tags blanked as `without_jinja` does, and with `server`, a server
/// template's as well: Handlebars' `{{{ … }}}` and each ERB, EJS or JSP
/// `<%= … %>` or `<%- … %>` read as a name, other `<% … %>` tags blanked.
fn without_tags(source: &str, server: bool) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut at = 0;
    while at + 1 < bytes.len() {
        let next = bytes.get(at + 2).copied();
        let (close, fill) = match (bytes[at], bytes[at + 1]) {
            (b'{', b'%') => ("%}", b' '),
            (b'{', b'#') => ("#}", b' '),
            (b'{', b'{') if server && next == Some(b'{') => ("}}}", b'_'),
            (b'{', b'{') => ("}}", b'_'),
            (b'<', b'%') if server => (
                "%>",
                if matches!(next, Some(b'=' | b'-')) {
                    b'_'
                } else {
                    b' '
                },
            ),
            _ => {
                at += 1;
                continue;
            }
        };
        let Some(length) = source[at + 2..].find(close) else {
            break;
        };
        let end = at + 2 + length + close.len();
        for byte in &mut out[at..end] {
            if *byte != b'\n' {
                *byte = fill;
            }
        }
        at = end;
    }
    String::from_utf8(out).unwrap_or_else(|_| source.to_string())
}

/// Whether a tree's syntax errors are few and small enough to judge the
/// rest of the file. Grammars miss some valid code: tree-sitter-typescript
/// reads a call signature that starts with `<T>` on the line after another
/// as its continuation, which left four of zustand's source files
/// unjudged, and tree-sitter-go flags a `const (…)` group closed on a raw
/// string's line. At most three error regions, an eighth of the source in
/// all; units holding an error are left out (`analysis::units`).
fn tolerable(root: Node<'_>, len: usize) -> bool {
    if !root.has_error() {
        return true;
    }
    if root.is_error() {
        return false;
    }
    let mut regions = Vec::new();
    error_regions(root, &mut regions);
    let bytes: usize = regions.iter().map(|r| r.len()).sum();
    regions.len() <= ERROR_REGIONS && bytes * ERROR_SHARE <= len
}

/// The smallest nodes that hold a syntax error.
fn error_regions(node: Node<'_>, out: &mut Vec<std::ops::Range<usize>>) {
    let mut cursor = node.walk();
    let holding: Vec<Node<'_>> = node
        .children(&mut cursor)
        .filter(|child| child.has_error())
        .collect();
    if holding.is_empty() || node.is_error() || node.is_missing() {
        out.push(node.byte_range());
        return;
    }
    for child in holding {
        error_regions(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locations::collect;
    use tree_sitter::{InputEdit, Point};

    #[test]
    fn jinja_tags_of_a_project_template_are_not_its_syntax() {
        let source = "{% if cookiecutter.use_celery == 'y' %}\nfrom celery import shared_task\n{% endif %}\nfrom {{ cookiecutter.project_slug }}.users.models import User\n\n\ndef total(values):\n    {# the café's sum #}\n    return sum(values)\n";
        let blanked = without_jinja(source);
        assert_eq!(blanked.len(), source.len());
        assert_eq!(blanked.lines().count(), source.lines().count());
        let placeholder = "_".repeat("{{ cookiecutter.project_slug }}".len());
        assert!(blanked.contains(&format!("from {placeholder}.users.models import User")));
        let tree = parse(
            Path::new("{{cookiecutter.project_slug}}/app/tasks.py"),
            source,
        )
        .unwrap()
        .unwrap();
        assert!(!tree.root_node().has_error());
        assert!(
            parse(Path::new("app/tasks.py"), source).is_err(),
            "outside a template its tags are syntax errors"
        );
    }

    #[test]
    fn identical_source_reuses_a_tree_across_paths() {
        let source = "function before() { return 1; }";
        let original = parse(Path::new("example.ts"), source).unwrap().unwrap();
        let repeated = parse(Path::new("other/example.ts"), source)
            .unwrap()
            .unwrap();
        assert_eq!(
            original.root_node().to_sexp(),
            repeated.root_node().to_sexp()
        );
    }

    #[test]
    fn changed_or_broken_source_is_parsed_again() {
        let path = Path::new("example.ts");
        let source = "function before() { return 1; }";
        parse(path, source).unwrap();
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
    }

    #[test]
    fn a_server_template_parses_as_its_inline_scripts_with_its_tags_blanked() {
        let erb = "<h1><%= @title %></h1>\n<% if admin? %><p>Admin</p><% end %>\n<script>\n  var name = \"<%= raw current_user.first_name %>\";\n  var tags = <%== @tags.to_json %>;\n  function greet() { document.write(name + location.hash) }\n</script>\n";
        let (masked, _) = crate::components::scripts("html", erb);
        let blanked = without_tags(&masked, true);
        assert_eq!(blanked.len(), erb.len());
        assert_eq!(blanked.lines().count(), erb.lines().count());
        assert!(!blanked.contains("<h1>") && !blanked.contains("<%"));
        assert!(blanked.contains(&format!(
            "var tags = {};",
            "_".repeat("<%== @tags.to_json %>".len())
        )));
        // Handlebars' triple stash and Jinja's tags, in a template directory.
        let jinja = "{% extends 'base.html' %}\n<script>\n  const user = {{{ user_json }}};\n  {% if debug %}console.log(user);{% endif %}\n  function show() { el.innerHTML = user.bio }\n</script>\n";
        for (path, source, function) in [
            ("app/views/sessions/new.html.erb", erb, ("greet", 6)),
            ("server/templates/profile.html", jinja, ("show", 5)),
        ] {
            let path = Path::new(path);
            assert!(
                !parse(path, source)
                    .unwrap()
                    .unwrap()
                    .root_node()
                    .has_error()
            );
            assert_eq!(
                collect(path, source, Path::new(".")).unwrap().1,
                vec![(function.0.into(), function.1)]
            );
        }
    }

    #[test]
    fn component_scripts_parse_in_place() {
        let astro = "---\nimport Layout from '../layouts/Layout.astro'\nconst posts = await getPosts()\nfunction title(p) { return p.data.title }\n---\n<Layout>{posts.map(p => <a>{title(p)}</a>)}</Layout>\n<script>\n  function toggle() { document.body.classList.toggle('dark') }\n</script>\n";
        assert_eq!(
            collect(Path::new("src/pages/index.astro"), astro, Path::new("."))
                .unwrap()
                .1,
            vec![("title".into(), 4), ("toggle".into(), 8)]
        );
        let vue = "<template>\n  <button @click=\"save\">{{ label }}</button>\n</template>\n<script setup lang=\"ts\">\nconst props = defineProps<{ label: string }>()\nfunction save(): void { emit('save') }\n</script>\n<style>.a { color: red }</style>\n";
        assert_eq!(
            collect(Path::new("Button.vue"), vue, Path::new("."))
                .unwrap()
                .1,
            vec![("save".into(), 6)]
        );
        let svelte = "<script>\n  let count = $state(0)\n  function increment() { count += 1 }\n</script>\n<script type=\"application/ld+json\">{\"a\": 1}</script>\n<button onclick={increment}>{count}</button>\n";
        let (masked, _) = crate::components::scripts("svelte", svelte);
        assert_eq!(masked.len(), svelte.len());
        assert!(!masked.contains("ld+json") && !masked.contains("<button"));
        assert_eq!(
            collect(Path::new("Counter.svelte"), svelte, Path::new("."))
                .unwrap()
                .1,
            vec![("increment".into(), 3)]
        );
    }

    #[test]
    fn the_extension_selects_the_grammar() {
        let jsx = "const view = <div/>;";
        assert!(parse(Path::new("view.tsx"), jsx).unwrap().is_some());
        assert!(parse(Path::new("view.ts"), jsx).is_err());
        assert!(parse(Path::new("view.txt"), jsx).unwrap().is_none());
        assert!(
            parse(Path::new("page.php"), "<?php echo $x; ?>\n<p>hi</p>\n")
                .unwrap()
                .is_some()
        );
        assert!(
            parse(Path::new("page.phtml"), "<p><?= $x ?></p>\n")
                .unwrap()
                .is_some()
        );
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
