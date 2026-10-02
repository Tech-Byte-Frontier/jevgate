//! Functions, methods and types of one file, with the local facts used for
//! grouping, callee and subject lookup, and marking bodies too small to judge.
mod callbacks;
mod facts;
mod generic;
mod import_names;
mod left_out;
mod ruby_definitions;
mod unit;
mod walk;

use super::{bend, is_comment, line_of, summary, text};
use anyhow::Result;
pub use left_out::LeftOut;
use std::{collections::BTreeSet, ops::Range, path::Path};
use tree_sitter::Node;
pub use unit::{Kind, MIN_BODY_LINES, Role, Unit};
use walk::{children, push, walk};

#[derive(Clone, Debug, Default)]
pub struct FileUnits {
    pub units: Vec<Unit>,
    pub imports: BTreeSet<String>,
    /// Top-level constants and bindings whose values hold literals.
    pub constants: Vec<super::literals::Constant>,
    /// Top-level statements that run when the module loads.
    pub setup: super::sites::Setup,
    /// False when no parser supports this language.
    pub parsed: bool,
    /// Read by the generic tier (`analysis::generic`): only function
    /// simplification, file organization, shared logic and comments judge it.
    pub generic: bool,
    /// Whether it is Django code: Python that imports Django or Django REST
    /// framework, or a Django settings module.
    pub django: bool,
    /// In Django code, the URL routes it declares.
    pub routes: Vec<super::django::Route>,
    /// In Django code, its upper-case module constants with their
    /// assignments as shown.
    pub module_constants: Vec<(String, String)>,
    /// In a server template, the code it runs while rendering that reads
    /// client data (`template_code`).
    pub template_code: super::sites::Setup,
    /// In Bend 2 code, the names that tell its proofs apart.
    bend: Option<BendNames>,
    /// Units whose syntax holds an error, left out of every rule, in source
    /// order (`left_out`).
    pub left_out: Vec<LeftOut>,
    /// Syntax errors outside every unit left out, as `syntax::error_regions`.
    errors: Vec<Range<usize>>,
}

/// A Bend 2 file's claims (laws a proof must hold), the laws that give a
/// def an `IO` type (`law main: IO(Unit)` above `def main():`) and import
/// aliases, and whether it is a file of proofs (`bend::proof_file`), to
/// tell its proofs from its code and its effects from pure code.
#[derive(Clone, Debug, Default)]
struct BendNames {
    claims: Vec<String>,
    effects: Vec<String>,
    aliases: Vec<String>,
    proofs: bool,
}

/// Units of a supported language. Unsupported languages return an unparsed,
/// empty result. A unit holding a syntax error is left out and recorded
/// (`left_out`); a file the parser could not read is an error, never an
/// empty clear file.
pub fn parse(path: &Path, source: &str) -> Result<FileUnits> {
    let Some(tree) = crate::syntax::parse(path, source)? else {
        return Ok(FileUnits::default());
    };
    let root = tree.root_node();
    let mut file = match super::generic::read(path, source) {
        Some(language) => tagged(language, root, source),
        None => walked(path, root, source),
    };
    calls_by_name(&mut file.units);
    if path.extension().is_none_or(|e| e != "rs") {
        for unit in &mut file.units {
            unit.passed.clear();
        }
    }
    Ok(file)
}

/// A file of the generic tier (`analysis::generic`): the definitions its
/// tag query captures, and the syntax errors outside them.
fn tagged(language: &super::generic::Language, root: Node<'_>, source: &str) -> FileUnits {
    let mut file = FileUnits {
        parsed: true,
        generic: true,
        ..Default::default()
    };
    generic::walk(language, root, source, &mut file);
    file.record_errors(root);
    file
}

/// A file of a language with an analyzer of its own: the units its walk
/// finds, then what the file holds outside them (`module_code`).
fn walked(path: &Path, root: Node<'_>, source: &str) -> FileUnits {
    let settings = super::django::settings_module(path, root, source);
    let mut file = FileUnits {
        parsed: true,
        django: settings || super::django::imports_django(path, root, source),
        bend: bend::file(path).then(|| bend_names(path, root, source)),
        ..Default::default()
    };
    walk(root, source, "", &mut file);
    file.record_errors(root);
    if let Some(names) = &file.bend {
        unaliased_calls(&mut file.units, &names.aliases);
    }
    module_code(path, (root, source), settings, &mut file);
    file
}

/// What a file holds outside its units: a Django module's routes and
/// constants, its module constants but those on lines syntax errors left
/// out, the statements that run outside every unit (a Django settings
/// module's with `settings`), and a server template's code.
fn module_code(
    path: &Path,
    (root, source): (Node<'_>, &str),
    settings: bool,
    file: &mut FileUnits,
) {
    if file.django {
        file.routes = super::django::routes(root, source);
        if !settings {
            file.module_constants = super::django::module_constants(root, source);
        }
    }
    file.constants = super::literals::constants(root, source);
    file.leave_out_constants(source);
    let spans: Vec<Range<usize>> = file.units.iter().map(|u| u.span.clone()).collect();
    file.setup = setup_of(path, root, source, &spans, settings);
    if crate::components::server_template(path) {
        file.template_code = super::template_code::template_code(path, source);
    }
}

/// The statements that run outside every unit: a framework configuration's
/// objects, a PHP page script, a server template's inline scripts, or a
/// module's setup (a Django settings module's with `settings`).
fn setup_of(
    path: &Path,
    root: Node<'_>,
    source: &str,
    spans: &[Range<usize>],
    settings: bool,
) -> super::sites::Setup {
    if framework_config(path) {
        super::sites::config_setup(root, source)
    } else if super::php::file(path) {
        super::sites::script(root, source, spans)
    } else if crate::components::server_template(path) {
        super::sites::inline_script(root, source, spans)
    } else {
        super::sites::setup(root, source, spans, settings)
    }
}

/// A Bend 2 file's claims and aliases. A law is a claim here when its
/// statement holds an equality or asks for a witness, or applies a def of
/// this file that computes a type, such as `Sorted(sort(xs))`.
fn bend_names(path: &Path, root: Node<'_>, source: &str) -> BendNames {
    let mut cursor = root.walk();
    let top: Vec<Node<'_>> = root.named_children(&mut cursor).collect();
    let propositions: BTreeSet<String> = top
        .iter()
        .filter(|n| n.kind() == "function_definition" && bend::type_level(**n))
        .map(|n| name_of(*n, source))
        .collect();
    let claims = top
        .iter()
        .filter(|n| bend::statement(**n, source).is_some_and(|s| s.claim(&propositions)))
        .map(|n| name_of(*n, source))
        .collect();
    let effects = top
        .iter()
        .filter(|n| bend::statement(**n, source).is_some_and(|s| s.head.as_deref() == Some("IO")))
        .map(|n| name_of(*n, source))
        .collect();
    BendNames {
        claims,
        effects,
        aliases: bend::aliases(root, source),
        proofs: bend::proof_file(path),
    }
}

/// A Bend 2 call through an import alias, `Sort.sort(xs)`, also calls the
/// def `sort` that the aliased file declares.
fn unaliased_calls(units: &mut [Unit], aliases: &[String]) {
    for unit in units {
        let unaliased: Vec<String> = unit
            .calls
            .iter()
            .filter_map(|call| {
                let (alias, name) = call.split_once('.')?;
                aliases.iter().any(|a| a == alias).then(|| name.to_string())
            })
            .collect();
        unit.calls.extend(unaliased);
    }
}

/// A function passed by name, such as `map(parse)`, is used like a call.
fn calls_by_name(units: &mut [Unit]) {
    let names: BTreeSet<String> = units.iter().map(|u| u.short_name.clone()).collect();
    for unit in units {
        let used: Vec<String> = unit
            .mentions
            .intersection(&names)
            .filter(|name| **name != unit.short_name)
            .cloned()
            .collect();
        unit.calls.extend(used);
        unit.mentions.clear();
    }
}

/// A framework configuration file whose settings are plain objects, such
/// as `next.config.mjs`.
fn framework_config(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| name.starts_with("next.config."))
}

fn name_of(node: Node<'_>, source: &str) -> String {
    node.child_by_field_name("name")
        .map(|n| text(n, source).to_string())
        .unwrap_or_default()
}

/// The syntax of one definition: the outer node that carries its documentation
/// (a declaration or decorator), the defining node, and its body if any.
#[derive(Clone, Copy)]
struct Definition<'t> {
    outer: Node<'t>,
    node: Node<'t>,
    body: Option<Node<'t>>,
}

impl<'t> Definition<'t> {
    /// A definition without a separate body, such as a type.
    fn whole(node: Node<'t>) -> Self {
        Self {
            outer: node,
            node,
            body: None,
        }
    }

    /// A function that `outer` defines as `node`, with the body of
    /// `function`: `node` itself, or the function a call around it wraps.
    fn function(outer: Node<'t>, node: Node<'t>, function: Node<'t>) -> Self {
        Self {
            outer,
            node,
            body: function.child_by_field_name("body"),
        }
    }
}

impl Unit {
    /// A unit placed in its file, before any fact about its code: its names
    /// and kind, its span with the documentation, comments and attributes
    /// above it, its lines, body, header and documentation line, and the
    /// size of its body.
    fn placed(
        definition: Definition<'_>,
        (short_name, owner): (&str, &str),
        kind: Kind,
        source: &str,
    ) -> Self {
        let Definition { outer, body, .. } = definition;
        let start = leading_start(outer);
        Self {
            name: if owner.is_empty() {
                short_name.to_string()
            } else {
                format!("{owner}::{short_name}")
            },
            short_name: short_name.to_string(),
            owner: owner.to_string(),
            kind,
            span: start..outer.end_byte(),
            line: line_of(source, outer.start_byte()),
            end_line: line_of(
                source,
                outer.end_byte().saturating_sub(1).max(outer.start_byte()),
            ),
            body: body.map(|b| b.byte_range()),
            signature: summary::signature(outer, body, source),
            doc: summary::doc_line(&source[start..outer.start_byte()], outer, source),
            body_lines: body.map_or(0, |b| body_lines(text(b, source))),
            nesting: 0,
            branch_chain: 0,
            blocks: Vec::new(),
            literals: Vec::new(),
            sites: Vec::new(),
            errors: Vec::new(),
            calls: BTreeSet::new(),
            passed: BTreeSet::new(),
            equality: false,
            routes: Vec::new(),
            refs: BTreeSet::new(),
            role: Role::Code,
            effects: false,
            joins_text: false,
            statement: None,
            mentions: BTreeSet::new(),
        }
    }
}

/// Whether a declaration is marked deprecated in the documentation and
/// attributes above it or in its header up to its body: a `@deprecated`
/// docblock tag, Java annotation or Python decorator, Rust's
/// `#[deprecated]`, C#'s `[Obsolete]`, or a comment line opening with
/// `Deprecated:` (Go, TomDoc). Without a body only what lies above it
/// counts: sqlmodel's module root, read whole, marked every class of a file
/// with one `@deprecated` method.
pub(crate) fn deprecated(node: Node<'_>, source: &str) -> bool {
    let end = node
        .child_by_field_name("definition")
        .unwrap_or(node)
        .child_by_field_name("body")
        .map_or(node.start_byte(), |body| body.start_byte());
    let header = source
        .get(leading_start(node)..end)
        .unwrap_or("")
        .to_ascii_lowercase();
    header.lines().map(str::trim_start).any(|line| {
        word(line, "@deprecated")
            || line.starts_with('@') && word(line, ".deprecated")
            || line.contains("#[deprecated")
            || line.contains("[obsolete")
            // A comment, not a parameter named `deprecated`.
            || line.strip_prefix(['/', '#', '*']).is_some_and(|comment| {
                comment
                    .trim_start_matches(['/', '*', '!'])
                    .trim_start()
                    .starts_with("deprecated:")
            })
    })
}

/// Whether `line` holds `name` as a whole word: `@deprecated`, not a mark
/// named `@deprecated_lifespan`.
fn word(line: &str, name: &str) -> bool {
    line.match_indices(name).any(|(at, _)| {
        !line[at + name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    })
}

/// Include documentation, comments and attributes directly above the definition.
fn leading_start(node: Node<'_>) -> usize {
    let mut start = node.start_byte();
    let mut previous = node.prev_named_sibling();
    // Ruby: comments above a class's first statement precede its body.
    if previous.is_none()
        && let Some(parent) = node.parent().filter(|p| p.kind() == "body_statement")
    {
        previous = parent.prev_named_sibling();
    }
    while let Some(sibling) = previous {
        if !(is_comment(sibling)
            || sibling.kind() == "attribute_item"
            || sibling.kind() == "decorator")
        {
            break;
        }
        start = sibling.start_byte();
        previous = sibling.prev_named_sibling();
    }
    start
}

/// Non-blank lines that are more than an opening or closing brace.
pub(crate) fn body_lines(body: &str) -> usize {
    body.lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line
                    .chars()
                    .all(|c| matches!(c, '{' | '}' | '(' | ')' | ';' | ','))
        })
        .count()
}

#[cfg(test)]
mod tests;
