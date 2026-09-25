//! Sites in a function body where a value reaches another program or a
//! setting is chosen: text built from values, calls and field assignments.
//! They are syntax only, never API names, and serve only as options for
//! locating a security finding; Jev judges what each one does.
use super::{django, is_comment, line_of, text};
use std::{collections::BTreeMap, ops::Range};
use tree_sitter::Node;

/// At most this many sites are offered per function, text-building first.
pub const MAX_SITES: usize = 16;
/// Longer site text is clipped; the function source still holds it whole.
const MAX_TEXT: usize = 160;
/// A statement longer than this many lines is shown as the site node alone.
const MAX_STATEMENT_LINES: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub id: String,
    pub text: String,
    pub line: usize,
    pub end_line: usize,
}

/// How directly a node shows a value reaching another program; lower first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Priority {
    /// A Django setting that decides security, such as `DEBUG = True`.
    SecuritySetting,
    /// In a Django settings module, an assignment to a subscript or field,
    /// such as `options["ssl_cert_reqs"] = None`: a setting passed on to a
    /// library, which built text and calls there, such as a URL joined
    /// with a path, crowded out.
    SettingsField,
    /// Text built from values: a template, f-string, concatenation or
    /// formatting macro.
    BuiltText,
    /// A call with an argument that is not a literal.
    CallWithValue,
    /// Another call, such as one that chooses a setting with literals.
    Call,
    /// An assignment to a field, property or subscript.
    FieldAssignment,
    /// Another Django setting, such as `LANGUAGE_CODE = 'en-us'`.
    Setting,
}

const LITERALS: &[&str] = &[
    "string",
    "string_literal",
    "interpreted_string_literal",
    "int_literal",
    "nil",
    "raw_string_literal",
    "number",
    "integer",
    "float",
    "integer_literal",
    "float_literal",
    "true",
    "false",
    "null",
    "none",
    "undefined",
    "boolean_literal",
    "null_literal",
    "character_literal",
    "real_literal",
    "verbatim_string_literal",
];

const STATEMENTS: &[&str] = &[
    "expression_statement",
    "lexical_declaration",
    "variable_declaration",
    "let_declaration",
    "return_statement",
    "return_expression",
    "short_var_declaration",
    "assignment_statement",
    "var_declaration",
    "defer_statement",
    "go_statement",
    "local_declaration_statement",
];

/// Rust's standard formatting macros build text from their arguments.
const FORMAT_MACROS: &[&str] = &[
    "format",
    "format_args",
    "write",
    "writeln",
    "print",
    "println",
    "eprint",
    "eprintln",
    "concat",
];

/// Go functions that build text from a format string and values.
const GO_FORMAT_CALLS: &[&str] = &["Sprintf", "Sprint", "Sprintln", "Errorf", "Fprintf"];

/// C# methods that build text from a format string or pieces.
const CSHARP_FORMAT_CALLS: &[&str] = &[
    "string.Format",
    "String.Format",
    "string.Concat",
    "String.Concat",
];

/// Sites of the body, one per statement, in source order with ids `S1…`.
/// In Django code (`django`), assignments to a subscript such as
/// `response['Location'] = url` are sites too.
pub fn in_node(body: Node<'_>, source: &str, django: bool) -> Vec<Site> {
    let mut best = BTreeMap::<usize, (Priority, Node<'_>)>::new();
    let mode = Mode {
        django,
        settings: false,
    };
    collect(body, body, source, mode, &mut best);
    numbered(best, source, &[])
}

/// What a file's sites include beyond the common ones.
#[derive(Clone, Copy)]
struct Mode {
    /// Django code: subscript assignments are sites.
    django: bool,
    /// A Django settings module: its setting assignments are sites.
    settings: bool,
}

fn numbered(
    best: BTreeMap<usize, (Priority, Node<'_>)>,
    source: &str,
    redactions: &[Range<usize>],
) -> Vec<Site> {
    let mut chosen: Vec<(Priority, Node<'_>)> = best.into_values().collect();
    chosen.sort_by_key(|(priority, node)| (*priority, node.start_byte()));
    chosen.truncate(MAX_SITES);
    chosen.sort_by_key(|(_, node)| node.start_byte());
    chosen
        .into_iter()
        .enumerate()
        .map(|(index, (_, node))| Site {
            id: format!("S{}", index + 1),
            text: clip(&django::redacted(source, node.byte_range(), redactions)),
            line: line_of(source, node.start_byte()),
            end_line: line_of(source, node.end_byte().saturating_sub(1)),
        })
        .collect()
}

/// Top-level statements that run when the module loads and call something,
/// such as `app.use(cors(options))`; definitions, imports and anything inside
/// a function unit are left out. In a Django settings module, statements
/// that assign settings (`DEBUG = True`) are setup too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Setup {
    /// Byte range, first and last line of each statement.
    pub statements: Vec<(Range<usize>, usize, usize)>,
    pub sites: Vec<Site>,
    /// Whether the module is a Django settings module.
    pub settings: bool,
    /// Contents of string literals assigned to secret names in a settings
    /// module, shown redacted.
    pub redactions: Vec<Range<usize>>,
    /// The settings a settings module assigns, in order, each with its
    /// assignment as shown (secret literals redacted).
    pub assigned: Vec<(String, String)>,
}

impl Setup {
    /// The text of one statement, with secret literals redacted.
    pub fn text(&self, source: &str, range: Range<usize>) -> String {
        django::redacted(source, range, &self.redactions)
    }
}

const SETUP_STATEMENTS: &[&str] = &[
    "expression_statement",
    "lexical_declaration",
    "variable_declaration",
    "if_statement",
    "with_statement",
    "local_declaration_statement",
];

/// The module's setup statements; `settings` for a Django settings module.
pub fn setup(root: Node<'_>, source: &str, units: &[Range<usize>], settings: bool) -> Setup {
    let mut statements = Vec::new();
    let mut redactions = Vec::new();
    let mut assigned = Vec::new();
    let mut best = BTreeMap::<usize, (Priority, Node<'_>)>::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        let node = match node.kind() {
            "export_statement" => match node.child_by_field_name("declaration") {
                Some(declaration) => declaration,
                None => continue,
            },
            // A C# top-level statement, as `Program.cs` writes its setup.
            "global_statement" => match node.named_child(0) {
                Some(statement) => statement,
                None => continue,
            },
            _ => node,
        };
        let range = node.byte_range();
        // Settings modules also choose values in `try` blocks, such as a
        // local override imported when present.
        let statement =
            SETUP_STATEMENTS.contains(&node.kind()) || settings && node.kind() == "try_statement";
        if !statement
            || units
                .iter()
                .any(|u| u.start < range.end && range.start < u.end)
            || !(calls_something(node) || settings && assigns_setting(node, source))
        {
            continue;
        }
        statements.push((
            range,
            line_of(source, node.start_byte()),
            line_of(source, node.end_byte().saturating_sub(1)),
        ));
        let mode = Mode {
            django: settings,
            settings,
        };
        collect(node, root, source, mode, &mut best);
        if settings {
            django::secret_literals(node, source, &mut redactions);
            let mut found = Vec::new();
            django::collect_settings(node, source, &mut found);
            assigned.extend(
                found.into_iter().map(|(name, range)| {
                    (name, clip(&django::redacted(source, range, &redactions)))
                }),
            );
        }
    }
    Setup {
        sites: numbered(best, source, &redactions),
        statements,
        settings,
        redactions,
        assigned,
    }
}

/// Whether a statement assigns a setting or changes part of one, directly
/// or inside its blocks.
fn assigns_setting(node: Node<'_>, source: &str) -> bool {
    match node.kind() {
        "expression_statement" => {
            django::setting_assigned(node, source).is_some()
                || django::setting_changed(node, source)
        }
        "if_statement" | "try_statement" | "with_statement" | "block" | "else_clause"
        | "elif_clause" | "except_clause" | "finally_clause" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .any(|c| assigns_setting(c, source))
        }
        _ => false,
    }
}

/// The setup of a framework configuration file such as `next.config.mjs`:
/// every top-level statement that holds an object literal, whether or not it
/// calls something, since settings like `headers()` are plain objects. Its
/// sites are the innermost objects (`{ key: 'Access-Control-Allow-Origin',
/// value: '*' }`) and the settings of the outermost object that hold a
/// single value, so a finding can point at the setting.
pub fn config_setup(root: Node<'_>, source: &str) -> Setup {
    let mut statements = Vec::new();
    let mut best = BTreeMap::<usize, (Priority, Node<'_>)>::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if !matches!(
            node.kind(),
            "expression_statement"
                | "lexical_declaration"
                | "variable_declaration"
                | "export_statement"
        ) || !holds_object(node)
        {
            continue;
        }
        statements.push((
            node.byte_range(),
            line_of(source, node.start_byte()),
            line_of(source, node.end_byte().saturating_sub(1)),
        ));
        settings(node, false, &mut best);
    }
    Setup {
        statements,
        sites: numbered(best, source, &[]),
        ..Setup::default()
    }
}

fn holds_object(node: Node<'_>) -> bool {
    node.kind() == "object" || {
        let mut cursor = node.walk();
        node.named_children(&mut cursor).any(holds_object)
    }
}

/// Innermost objects, and single-value properties of outermost objects.
fn settings<'t>(
    node: Node<'t>,
    inside_object: bool,
    best: &mut BTreeMap<usize, (Priority, Node<'t>)>,
) {
    if is_comment(node) {
        return;
    }
    if node.kind() == "object" {
        if !holds_object_below(node) {
            best.entry(node.start_byte())
                .or_insert((Priority::FieldAssignment, node));
            return;
        }
        let mut cursor = node.walk();
        for property in node.named_children(&mut cursor) {
            let value = property.child_by_field_name("value");
            match value {
                Some(value) if !inside_object && !holds_object(value) => {
                    best.entry(property.start_byte())
                        .or_insert((Priority::FieldAssignment, property));
                }
                _ => settings(property, true, best),
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        settings(child, inside_object, best);
    }
}

fn holds_object_below(object: Node<'_>) -> bool {
    let mut cursor = object.walk();
    object.named_children(&mut cursor).any(holds_object)
}

fn calls_something(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "call_expression"
            | "call"
            | "new_expression"
            | "macro_invocation"
            | "invocation_expression"
            | "object_creation_expression"
    ) || {
        let mut cursor = node.walk();
        node.named_children(&mut cursor).any(calls_something)
    }
}

fn collect<'t>(
    node: Node<'t>,
    body: Node<'t>,
    source: &str,
    mode: Mode,
    best: &mut BTreeMap<usize, (Priority, Node<'t>)>,
) {
    if is_comment(node) {
        return;
    }
    if let Some(priority) = priority(node, source, mode) {
        let shown = statement(node, body, source);
        let entry = best.entry(shown.start_byte()).or_insert((priority, shown));
        entry.0 = entry.0.min(priority);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect(child, body, source, mode, best);
    }
}

fn priority(node: Node<'_>, source: &str, mode: Mode) -> Option<Priority> {
    match node.kind() {
        "assignment" if mode.settings => {
            let left = node.child_by_field_name("left")?;
            let name = text(left, source);
            if left.kind() != "identifier" || !django::setting_name(name) {
                return field_assignment(node, mode);
            }
            Some(if django::security_setting(name) {
                Priority::SecuritySetting
            } else {
                Priority::Setting
            })
        }
        "template_string" if has_child(node, "template_substitution") => Some(Priority::BuiltText),
        // React's raw-markup property: `dangerouslySetInnerHTML={{ __html: value }}`.
        "jsx_attribute"
            if node
                .named_child(0)
                .is_some_and(|name| text(name, source) == "dangerouslySetInnerHTML") =>
        {
            Some(Priority::BuiltText)
        }
        "string" | "interpolated_string_expression" if has_child(node, "interpolation") => {
            Some(Priority::BuiltText)
        }
        "binary_expression" | "binary_operator" if concatenates(node, source) => {
            Some(Priority::BuiltText)
        }
        "macro_invocation" => node
            .child_by_field_name("macro")
            .map(|name| text(name, source).rsplit("::").next().unwrap_or(""))
            .filter(|name| FORMAT_MACROS.contains(name))
            .map(|_| Priority::BuiltText),
        "call_expression"
            if node
                .child_by_field_name("function")
                .filter(|f| f.kind() == "selector_expression")
                .and_then(|f| f.child_by_field_name("field"))
                .is_some_and(|f| GO_FORMAT_CALLS.contains(&text(f, source))) =>
        {
            Some(Priority::BuiltText)
        }
        "invocation_expression"
            if node
                .child_by_field_name("function")
                .is_some_and(|f| CSHARP_FORMAT_CALLS.contains(&text(f, source))) =>
        {
            Some(Priority::BuiltText)
        }
        "call_expression"
        | "call"
        | "new_expression"
        | "invocation_expression"
        | "object_creation_expression" => Some(
            if node
                .child_by_field_name("arguments")
                .is_some_and(|args| has_value(args))
            {
                Priority::CallWithValue
            } else {
                Priority::Call
            },
        ),
        "assignment_expression" | "assignment" | "augmented_assignment_expression" => {
            field_assignment(node, mode)
        }
        _ => None,
    }
}

/// An assignment to a field or property, or in Django code to a Python
/// subscript, such as `response['Location'] = url` or
/// `options["ssl_cert_reqs"] = None`.
fn field_assignment(node: Node<'_>, mode: Mode) -> Option<Priority> {
    let field = node
        .child_by_field_name("left")
        .is_some_and(|left| match left.kind() {
            "member_expression"
            | "attribute"
            | "field_expression"
            | "subscript_expression"
            | "member_access_expression"
            | "element_access_expression" => true,
            "subscript" => mode.django,
            _ => false,
        })
        || node
            .parent()
            .is_some_and(|p| p.kind() == "initializer_expression");
    field.then_some(if mode.settings {
        Priority::SettingsField
    } else {
        Priority::FieldAssignment
    })
}

fn has_child(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).any(|c| c.kind() == kind)
}

/// `+` or `%` joining a string literal with something that is not a literal.
fn concatenates(node: Node<'_>, source: &str) -> bool {
    let operator = node
        .child_by_field_name("operator")
        .map_or("", |o| text(o, source));
    let (Some(left), Some(right)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return false;
    };
    let string = |n: Node<'_>| {
        matches!(
            n.kind(),
            "string"
                | "string_literal"
                | "template_string"
                | "interpreted_string_literal"
                | "raw_string_literal"
                | "verbatim_string_literal"
                | "interpolated_string_expression"
        )
    };
    matches!(operator, "+" | "%")
        && (string(left) || string(right))
        && !(LITERALS.contains(&left.kind()) && LITERALS.contains(&right.kind()))
}

/// Whether an argument list holds something other than literals.
fn has_value(arguments: Node<'_>) -> bool {
    let mut cursor = arguments.walk();
    arguments.named_children(&mut cursor).any(|argument| {
        let value = match argument.kind() {
            "keyword_argument" => argument.child_by_field_name("value"),
            // A C# argument wraps its expression, after any `name:`.
            "argument" => {
                argument.named_child(argument.named_child_count().saturating_sub(1) as u32)
            }
            _ => Some(argument),
        };
        value.is_some_and(|v| !LITERALS.contains(&v.kind()) && !is_comment(v))
    })
}

/// The enclosing statement, so a site reads in context; the node itself when
/// the statement is long or the body is reached first.
fn statement<'t>(node: Node<'t>, body: Node<'t>, source: &str) -> Node<'t> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.id() == body.id() {
            break;
        }
        if STATEMENTS.contains(&parent.kind()) {
            let lines = line_of(source, parent.end_byte()) - line_of(source, parent.start_byte());
            return if lines < MAX_STATEMENT_LINES {
                parent
            } else {
                node
            };
        }
        current = parent;
    }
    node
}

pub(crate) fn clip(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_TEXT {
        return collapsed;
    }
    format!("{}…", collapsed.chars().take(MAX_TEXT).collect::<String>())
}

#[cfg(test)]
mod tests {
    use crate::analysis::units::parse;
    use std::path::Path;

    fn sites(path: &str, source: &str) -> Vec<(String, usize)> {
        let file = parse(Path::new(path), source).unwrap();
        file.units[0]
            .sites
            .iter()
            .map(|s| (s.text.clone(), s.line))
            .collect()
    }

    #[test]
    fn built_text_and_calls_are_sites_in_every_language() {
        let python = sites(
            "a.py",
            "def find(name):\n    sql = f\"SELECT id FROM users WHERE name = '{name}'\"\n    rows = db.execute(sql)\n    log(\"done\")\n    return rows\n",
        );
        assert_eq!(
            python.iter().map(|(_, line)| *line).collect::<Vec<_>>(),
            [2, 3, 4]
        );
        assert!(python[0].0.starts_with("sql = f\"SELECT"));
        let script = sites(
            "a.ts",
            "function run(file: string) {\n  exec(`convert ${file}`, done);\n  el.innerHTML = file;\n}\n",
        );
        assert_eq!(
            script,
            [
                ("exec(`convert ${file}`, done);".to_string(), 2),
                ("el.innerHTML = file;".to_string(), 3)
            ]
        );
        let rust = sites(
            "a.rs",
            "fn load(conn: &Connection, table: &str) -> Result<()> {\n    let sql = format!(\"SELECT * FROM {table}\");\n    conn.execute(&sql, [])?;\n    Ok(())\n}\n",
        );
        assert_eq!(rust.len(), 3, "{rust:?}");
        assert!(rust[0].0.starts_with("let sql = format!"));
    }

    #[test]
    fn subscript_assignments_are_sites_only_in_django_code() {
        let body = "def download(request):\n    response = HttpResponse()\n    response['Location'] = request.GET['next']\n    return response\n";
        let plain = sites("a.py", body);
        assert!(!plain.iter().any(|(_, line)| *line == 3), "{plain:?}");
        let django = sites(
            "a.py",
            &format!("from django.http import HttpResponse\n\n{body}"),
        );
        assert!(
            django
                .iter()
                .any(|(text, _)| text == "response['Location'] = request.GET['next']"),
            "{django:?}"
        );
    }

    #[test]
    fn a_settings_module_is_setup_with_its_security_settings_first_and_secrets_redacted() {
        let source = "import os\n\nLANGUAGE_CODE = 'en-us'\nSECRET_KEY = 'abc123'\nDEBUG = True\nINSTALLED_APPS = ['shop']\n\ntry:\n    from .local import *\nexcept ImportError:\n    pass\n";
        let file = parse(Path::new("shop/settings.py"), source).unwrap();
        assert!(file.setup.settings && file.django);
        let texts: Vec<&str> = file.setup.sites.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            texts[..3],
            [
                "LANGUAGE_CODE = 'en-us'",
                "SECRET_KEY = '<redacted 6-character literal>'",
                "DEBUG = True"
            ]
        );
        assert!(!texts.iter().any(|t| t.contains("abc123")));
        // Past the site limit, security settings are the ones kept.
        let many: String = (0..super::MAX_SITES)
            .map(|i| format!("OPTION_{i} = {i}\n"))
            .chain(["DEBUG = True\n".to_string()])
            .collect();
        let crowded = parse(Path::new("shop/settings.py"), &many).unwrap();
        assert_eq!(crowded.setup.sites.len(), super::MAX_SITES);
        assert_eq!(crowded.setup.sites.last().unwrap().text, "DEBUG = True");
        assert_eq!(
            file.setup.statements.len(),
            4,
            "every setting, but not the try block"
        );
        assert_eq!(
            file.setup
                .assigned
                .iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>(),
            ["LANGUAGE_CODE", "SECRET_KEY", "DEBUG", "INSTALLED_APPS"]
        );
        // A setting passed on to a library outranks built text and calls.
        let crowded: String = (0..super::MAX_SITES)
            .map(|i| format!("URL_{i} = os.environ.get('URL') + '/{i}'\n"))
            .chain(["DEBUG = False\nOPTIONS['ssl_cert_reqs'] = None\n".to_string()])
            .collect();
        let crowded = parse(Path::new("shop/settings.py"), &crowded).unwrap();
        assert!(
            crowded
                .setup
                .sites
                .iter()
                .any(|s| s.text == "OPTIONS['ssl_cert_reqs'] = None")
        );
        let plain = parse(Path::new("shop/constants.py"), "DEBUG = True\nLIMIT = 3\n").unwrap();
        assert!(!plain.setup.settings && plain.setup.statements.is_empty());
    }

    #[test]
    fn module_setup_holds_top_level_calls_outside_functions() {
        let source = "import express from 'express'\nconst app = express()\napp.use(cors({ origin: '*', credentials: true }))\nconst limit = 10\nexport const handler = (req) => send(req)\n";
        let file = parse(Path::new("server.ts"), source).unwrap();
        let lines: Vec<usize> = file.setup.statements.iter().map(|s| s.1).collect();
        assert_eq!(lines, [2, 3]);
        assert_eq!(file.setup.sites.len(), 2);
        assert!(file.setup.sites[1].text.starts_with("app.use(cors("));
        let python = parse(
            Path::new("main.py"),
            "app = FastAPI()\napp.add_middleware(CORSMiddleware, allow_origins=['*'])\n\ndef run():\n    serve(app)\n",
        )
        .unwrap();
        assert_eq!(python.setup.statements.len(), 2);
    }

    #[test]
    fn react_raw_markup_properties_are_sites() {
        let found = sites(
            "page.tsx",
            "function Post({ post }: Props) {\n  const title = post.title;\n  return (\n    <article>\n      <h1>{title}</h1>\n      <div dangerouslySetInnerHTML={{ __html: post.body }} />\n    </article>\n  );\n}\n",
        );
        assert_eq!(
            found,
            [(
                "dangerouslySetInnerHTML={{ __html: post.body }}".to_string(),
                6
            )]
        );
    }

    #[test]
    fn next_config_settings_are_module_setup_without_calls() {
        let source = "import type { NextConfig } from 'next';\n\nconst nextConfig: NextConfig = {\n  reactStrictMode: true,\n  images: { remotePatterns: [{ protocol: 'https', hostname: '**' }] },\n  async headers() {\n    return [\n      {\n        source: '/api/:path*',\n        headers: [{ key: 'Access-Control-Allow-Origin', value: '*' }],\n      },\n    ];\n  },\n};\n\nexport default nextConfig;\n";
        let file = parse(Path::new("next.config.ts"), source).unwrap();
        let lines: Vec<(usize, usize)> = file.setup.statements.iter().map(|s| (s.1, s.2)).collect();
        assert_eq!(
            lines,
            [(3, 14)],
            "the config object, not the import or export"
        );
        let texts: Vec<&str> = file.setup.sites.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "reactStrictMode: true",
                "{ protocol: 'https', hostname: '**' }",
                "{ key: 'Access-Control-Allow-Origin', value: '*' }",
            ]
        );
        // The same object elsewhere is ordinary code without setup calls.
        let other = parse(Path::new("config.ts"), source).unwrap();
        assert!(other.setup.statements.is_empty());
    }

    #[test]
    fn built_text_is_kept_first_when_a_body_has_many_calls() {
        let mut body = String::from("def many(value):\n");
        for i in 0..30 {
            body.push_str(&format!("    step_{i}(value)\n"));
        }
        body.push_str("    run(\"echo \" + value)\n");
        let found = sites("a.py", &body);
        assert_eq!(found.len(), super::MAX_SITES);
        assert_eq!(found.last().unwrap().0, "run(\"echo \" + value)");
    }
}
