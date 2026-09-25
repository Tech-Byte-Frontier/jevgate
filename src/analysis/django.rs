//! Django conventions read from syntax: settings modules, whose top-level
//! assignments configure the deployed site, and the literals in them that
//! hold secrets, which are shown redacted.
use super::text;
use std::{ops::Range, path::Path};
use tree_sitter::Node;

/// Settings only a Django settings module assigns; one of them marks a
/// module as settings wherever it lives.
const PROJECT_SETTINGS: [&str; 4] = [
    "INSTALLED_APPS",
    "ROOT_URLCONF",
    "MIDDLEWARE",
    "MIDDLEWARE_CLASSES",
];

/// Settings that decide how the deployed site protects itself; in a module
/// under a `settings` directory or named `settings.py`, one of them marks it
/// as settings, such as a `dev.py` that only turns `DEBUG` on.
const SECURITY_SETTINGS: [&str; 12] = [
    "DEBUG",
    "SECRET_KEY",
    "ALLOWED_HOSTS",
    "PASSWORD_HASHERS",
    "SESSION_",
    "CSRF_",
    "SECURE_",
    "CORS_",
    "X_FRAME_OPTIONS",
    "REST_FRAMEWORK",
    "JWT_",
    "SIMPLE_JWT",
];

/// Whether a setting name decides security: the offered sites put them first.
pub fn security_setting(name: &str) -> bool {
    SECURITY_SETTINGS.iter().any(|s| {
        if s.ends_with('_') {
            name.starts_with(s)
        } else {
            name == *s
        }
    })
}

/// Whether a Python module is a Django settings module: it assigns a
/// project setting such as `INSTALLED_APPS`, or it is named `settings.py` or
/// sits in a `settings` directory and assigns a security setting or a
/// secret, such as `JWT_AUTH` or `GOOGLE_OAUTH2_CLIENT_SECRET`.
pub fn settings_module(path: &Path, root: Node<'_>, source: &str) -> bool {
    if path.extension().is_none_or(|e| e != "py") {
        return false;
    }
    let names = assigned_settings(root, source);
    if names.iter().any(|n| PROJECT_SETTINGS.contains(&n.as_str())) {
        return true;
    }
    let named = path.file_stem().is_some_and(|s| s == "settings")
        || path
            .parent()
            .is_some_and(|p| p.components().any(|c| c.as_os_str() == "settings"))
        || star_import(root, source);
    named && names.iter().any(|n| security_setting(n) || secret_name(n))
}

/// Packages whose import marks a Python file as Django code.
const DJANGO_PACKAGES: [&str; 2] = ["django", "rest_framework"];

/// Whether a Python file imports Django or Django REST framework, at the top
/// level or inside a top-level `try` or `if` block.
pub fn imports_django(path: &Path, root: Node<'_>, source: &str) -> bool {
    if path.extension().is_none_or(|e| e != "py") {
        return false;
    }
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .any(|node| imports_package(node, source))
}

fn imports_package(node: Node<'_>, source: &str) -> bool {
    match node.kind() {
        "import_statement" | "import_from_statement" => {
            let module = if node.kind() == "import_from_statement" {
                node.child_by_field_name("module_name")
            } else {
                node.named_child(0)
            };
            module.is_some_and(|m| {
                let name = text(m, source);
                let package = name.split(['.', ' ']).next().unwrap_or(name);
                DJANGO_PACKAGES.contains(&package)
            })
        }
        "if_statement" | "try_statement" | "block" | "else_clause" | "elif_clause"
        | "except_clause" | "finally_clause" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .any(|c| imports_package(c, source))
        }
        _ => false,
    }
}

/// Whether the module imports every name of another (`from .base import *`),
/// as settings for one environment extend the shared ones.
fn star_import(root: Node<'_>, source: &str) -> bool {
    let mut cursor = root.walk();
    root.named_children(&mut cursor).any(|node| {
        node.kind() == "import_from_statement" && text(node, source).trim_end().ends_with('*')
    })
}

/// Whether a module's source imports every name of the module at `target`
/// (`from .base import *`, `from config.settings.cors import *`), named by
/// its last dotted part: settings for one environment extend shared ones.
pub fn extends(source: &str, target: &Path) -> bool {
    let name = module_name(target);
    !name.is_empty()
        && source.lines().any(|line| {
            let code = line.split('#').next().unwrap_or("").trim();
            let Some(module) = code
                .strip_prefix("from ")
                .and_then(|rest| rest.strip_suffix("*"))
                .and_then(|rest| rest.trim_end().strip_suffix("import"))
            else {
                return false;
            };
            module.trim().rsplit('.').next() == Some(name)
        })
}

/// A URLconf entry that routes requests to a view, such as
/// `path('tasks/<int:task_id>/', views.task_edit)`: it shows that the view
/// receives outside requests and what its URL parameters may hold.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    /// The view's name: a function, or the class of a class-based view or
    /// Django REST framework viewset.
    pub view: String,
    /// The module part before the name, such as `views` in `views.index`.
    pub module: Option<String>,
    pub line: usize,
    /// The whole entry on one line.
    pub text: String,
}

/// Calls that route URLs to views: `path`, `re_path`, `url`, and a router's
/// `register`.
const ROUTE_CALLS: [&str; 4] = ["path", "re_path", "url", "register"];

/// The URL routes a Django file declares, wherever the calls are.
pub fn routes(root: Node<'_>, source: &str) -> Vec<Route> {
    let mut found = Vec::new();
    collect_routes(root, source, &mut found);
    found
}

fn collect_routes(node: Node<'_>, source: &str, found: &mut Vec<Route>) {
    if node.kind() == "call"
        && let Some(route) = route(node, source)
    {
        found.push(route);
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_routes(child, source, found);
    }
}

/// A call to a route function whose first argument is a string pattern and
/// whose next one names a view.
fn route(call: Node<'_>, source: &str) -> Option<Route> {
    let function = super::callee_name(call.child_by_field_name("function")?, source)?;
    if !ROUTE_CALLS.contains(&function.as_str()) {
        return None;
    }
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    let positional: Vec<Node<'_>> = arguments
        .named_children(&mut cursor)
        .filter(|a| a.kind() != "keyword_argument" && a.kind() != "comment")
        .collect();
    if positional.first()?.kind() != "string" {
        return None;
    }
    let (module, view) = view_name(*positional.get(1)?, source)?;
    Some(Route {
        view,
        module,
        line: super::line_of(source, call.start_byte()),
        text: super::sites::clip(text(call, source)),
    })
}

/// The view an argument names: `views.index`, `index`,
/// `views.TaskView.as_view()`, or the dotted path `'app.views.index'`.
fn view_name(argument: Node<'_>, source: &str) -> Option<(Option<String>, String)> {
    let dotted = match argument.kind() {
        "identifier" | "attribute" => text(argument, source).to_string(),
        "call" => {
            let function = argument.child_by_field_name("function")?;
            let object = function.child_by_field_name("object")?;
            if function.kind() != "attribute"
                || text(function.child_by_field_name("attribute")?, source) != "as_view"
            {
                return None;
            }
            text(object, source).to_string()
        }
        "string" => literal_content(argument).map(|r| source[r].to_string())?,
        _ => return None,
    };
    let mut parts = dotted.rsplit('.');
    let view = parts.next()?.trim().to_string();
    let module = parts.next().map(|m| m.trim().to_string());
    (!view.is_empty() && view.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .then_some((module, view))
}

/// Methods Django and Django REST framework call on a class-based view or
/// viewset to answer a request.
const VIEW_METHODS: [&str; 16] = [
    "get",
    "post",
    "put",
    "patch",
    "delete",
    "dispatch",
    "list",
    "create",
    "retrieve",
    "update",
    "partial_update",
    "destroy",
    "form_valid",
    "get_queryset",
    "get_object",
    "get_context_data",
];

/// Whether a route reaches a unit of the module at `path`: a function of
/// that name, or a request method of a class of that name. A route that
/// names a module (`views.index`) must name this one.
pub fn routes_to(route: &Route, path: &Path, short_name: &str, owner: &str) -> bool {
    let name = if owner.is_empty() {
        short_name
    } else if VIEW_METHODS.contains(&short_name) {
        owner
    } else {
        return false;
    };
    route.view == name
        && route
            .module
            .as_deref()
            .is_none_or(|m| m == module_name(path))
}

/// The name a module is imported by: its file stem, or its package's name
/// for an `__init__.py`.
fn module_name(path: &Path) -> &str {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem == "__init__" {
        path.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
    } else {
        stem
    }
}

/// The command name of a Django management command module
/// (`app/management/commands/<name>.py`), which a person runs by hand with
/// `manage.py <name>`.
pub fn management_command(path: &Path) -> Option<&str> {
    let parts: Vec<&str> = path.iter().filter_map(|p| p.to_str()).collect();
    let [.., management, commands, _] = parts.as_slice() else {
        return None;
    };
    let name = path.file_stem()?.to_str()?;
    (*management == "management" && *commands == "commands" && !name.starts_with('_'))
        .then_some(name)
}

/// Upper-case module constants of a Django file, such as
/// `FIXTURE_DIR = Path(settings.BASE_DIR) / "fixtures"`, each with its
/// assignment as shown (secret literals redacted): a function that builds a
/// path or query from one is shown what it holds.
pub fn module_constants(root: Node<'_>, source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut redactions = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        if node.kind() == "expression_statement" {
            secret_literals(node, source, &mut redactions);
            collect_settings(node, source, &mut found);
        }
    }
    found
        .into_iter()
        .map(|(name, range)| {
            (
                name,
                super::sites::clip(&redacted(source, range, &redactions)),
            )
        })
        .collect()
}

/// Upper-case names assigned at the top level, including inside `if` and
/// `try` blocks, as settings modules choose values per environment.
fn assigned_settings(root: Node<'_>, source: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        collect_settings(node, source, &mut found);
    }
    found.into_iter().map(|(name, _)| name).collect()
}

/// Settings a top-level statement assigns, directly or inside its blocks,
/// each with the byte range of its assignment statement.
pub fn collect_settings(node: Node<'_>, source: &str, found: &mut Vec<(String, Range<usize>)>) {
    match node.kind() {
        "expression_statement" => {
            if let Some(name) = setting_assigned(node, source) {
                found.push((name.to_string(), node.byte_range()));
            }
        }
        "if_statement" | "try_statement" | "block" | "else_clause" | "elif_clause"
        | "except_clause" | "finally_clause" | "with_statement" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_settings(child, source, found);
            }
        }
        _ => {}
    }
}

/// The setting an expression statement assigns, such as `DEBUG` in
/// `DEBUG = True` or `SECRET_KEY: str = …`.
pub fn setting_assigned<'s>(statement: Node<'_>, source: &'s str) -> Option<&'s str> {
    let assignment = statement.named_child(0)?;
    if !matches!(assignment.kind(), "assignment" | "augmented_assignment") {
        return None;
    }
    let left = assignment.child_by_field_name("left")?;
    let name = text(left, source);
    (left.kind() == "identifier" && setting_name(name)).then_some(name)
}

/// Whether an expression statement changes part of a setting, such as
/// `CACHES["default"]["OPTIONS"]["ssl_cert_reqs"] = None`.
pub fn setting_changed(statement: Node<'_>, source: &str) -> bool {
    let Some(assignment) = statement
        .named_child(0)
        .filter(|a| a.kind() == "assignment")
    else {
        return false;
    };
    let mut target = assignment.child_by_field_name("left");
    while let Some(node) = target {
        match node.kind() {
            "subscript" => target = node.child_by_field_name("value"),
            "attribute" => target = node.child_by_field_name("object"),
            "identifier" => {
                return node != assignment.child_by_field_name("left").unwrap()
                    && setting_name(text(node, source));
            }
            _ => return false,
        }
    }
    false
}

/// An upper-case name such as `SESSION_COOKIE_SECURE`.
pub fn setting_name(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Parts of a name that mark its value as a secret.
const SECRET_PARTS: [&str; 6] = [
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "TOKEN",
    "PRIVATE",
    "CREDENTIAL",
];

/// Whether a setting or dictionary key names a secret, such as `SECRET_KEY`,
/// `'PASSWORD'` or `AWS_SECRET_ACCESS_KEY`; `…_KEY` counts too.
pub fn secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_PARTS.iter().any(|p| upper.contains(p)) || upper.ends_with("_KEY") || upper == "KEY"
}

/// Byte ranges of the contents of string literals assigned to secret names
/// in these statements: `SECRET_KEY = '…'` and `'PASSWORD': '…'` in a
/// dictionary. They are shown redacted: the literal is evidence that the
/// secret is fixed in code; its value is never uploaded.
pub fn secret_literals(node: Node<'_>, source: &str, found: &mut Vec<Range<usize>>) {
    let value = match node.kind() {
        "assignment" => node
            .child_by_field_name("left")
            .filter(|l| l.kind() == "identifier" && secret_name(text(*l, source)))
            .and_then(|_| node.child_by_field_name("right")),
        "pair" => node
            .child_by_field_name("key")
            .filter(|k| k.kind() == "string" && secret_name(string_content(*k, source)))
            .and_then(|_| node.child_by_field_name("value")),
        "keyword_argument" => node
            .child_by_field_name("name")
            .filter(|n| secret_name(text(*n, source)))
            .and_then(|_| node.child_by_field_name("value")),
        _ => None,
    };
    if let Some(range) = value
        .and_then(literal_content)
        .filter(|r| !dotted_path(&source[r.clone()]))
    {
        found.push(range);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        secret_literals(child, source, found);
    }
}

/// A dotted Python path such as `app.auth.services.get_secret_key`: a
/// setting that names the function or class holding a secret, not a secret.
fn dotted_path(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() >= 2
        && parts.iter().all(|part| {
            part.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && part.chars().all(|c| c.is_alphanumeric() || c == '_')
        })
}

/// The text between the quotes of a string literal without interpolation.
fn string_content<'s>(node: Node<'_>, source: &'s str) -> &'s str {
    literal_content(node).map_or("", |r| &source[r])
}

/// The byte range between the quotes of a plain, non-empty string literal.
fn literal_content(node: Node<'_>) -> Option<Range<usize>> {
    if node.kind() != "string" {
        return None;
    }
    let mut cursor = node.walk();
    let parts: Vec<Node<'_>> = node.named_children(&mut cursor).collect();
    if parts.iter().any(|p| p.kind() == "interpolation") {
        return None;
    }
    let content = parts.iter().find(|p| p.kind() == "string_content")?;
    Some(content.byte_range()).filter(|r| !r.is_empty())
}

/// `source[range]` with each redacted literal inside it replaced by a note
/// of its length, so a secret's presence shows but its value does not.
pub fn redacted(source: &str, range: Range<usize>, redactions: &[Range<usize>]) -> String {
    let mut shown = String::new();
    let mut at = range.start;
    for secret in redactions
        .iter()
        .filter(|r| range.start <= r.start && r.end <= range.end)
    {
        if secret.start < at {
            continue;
        }
        shown.push_str(&source[at..secret.start]);
        let length = source[secret.clone()].chars().count();
        shown.push_str(&format!("<redacted {length}-character literal>"));
        at = secret.end;
    }
    shown.push_str(&source[at..range.end]);
    shown
}

/// A line outside a settings module that names it as the settings to run
/// with, such as `ENV DJANGO_SETTINGS_MODULE=site.settings.production` in a
/// Dockerfile or `os.environ.setdefault(…, "site.settings.dev")` in
/// `manage.py`: whether a module is deployed or only local shows there.
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    pub file: std::path::PathBuf,
    pub line: usize,
    pub text: String,
}

/// Longest line text kept for a selection.
const SELECTION_TEXT: usize = 160;
/// Files larger than this are not searched for selections.
pub const SELECTION_FILE_BYTES: u64 = 65_536;
/// Selections shown per settings module, at most.
pub const SELECTIONS: usize = 8;

/// Whether a file may name the settings module a process runs with:
/// container, process, CI, test runner and shell configuration, and the
/// Python entry points Django projects keep beside their settings. Files
/// named `.env…` are left out, since their other lines hold secrets.
pub fn selection_file(relative: &Path) -> bool {
    let name = relative
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name.starts_with(".env") {
        return false;
    }
    let extension = relative
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    name.starts_with("Dockerfile")
        || matches!(
            name,
            "Procfile" | "Makefile" | "manage.py" | "wsgi.py" | "asgi.py" | "conftest.py"
        )
        || matches!(
            extension,
            "yml" | "yaml" | "toml" | "cfg" | "ini" | "sh" | "json"
        )
}

/// The lines of one file that set the settings module to run with.
pub fn selections_in(relative: &Path, text: &str) -> Vec<Selection> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains("DJANGO_SETTINGS_MODULE") || line.contains("--settings"))
        .map(|(index, line)| Selection {
            file: relative.to_path_buf(),
            line: index + 1,
            text: super::sites::clip(line.trim())
                .chars()
                .take(SELECTION_TEXT)
                .collect(),
        })
        .collect()
}

/// The selections that name the settings module at `relative` by its dotted
/// module path (`site.settings.production`, or from its last two parts).
pub fn selected_by<'s>(relative: &Path, selections: &'s [Selection]) -> Vec<&'s Selection> {
    let module = relative.with_extension("");
    let parts: Vec<&str> = module.iter().filter_map(|p| p.to_str()).collect();
    let shortest = parts.len().min(2);
    let names: Vec<String> = (0..=parts.len() - shortest)
        .map(|start| parts[start..].join("."))
        .collect();
    selections
        .iter()
        .filter(|s| names.iter().any(|name| names_module(&s.text, name)))
        .take(SELECTIONS)
        .collect()
}

/// `name` appears as a whole dotted module path, not as part of a longer one.
fn names_module(line: &str, name: &str) -> bool {
    let part = |c: char| c.is_alphanumeric() || c == '_' || c == '.';
    line.match_indices(name).any(|(start, _)| {
        line[..start].chars().next_back().is_none_or(|c| !part(c))
            && line[start + name.len()..]
                .chars()
                .next()
                .is_none_or(|c| !part(c))
    })
}

/// A Django template that writes values without escaping them: `|safe`,
/// `|safeseq` or `{% autoescape off %}`. A view that renders it by name is
/// sent with those lines, since the markup a view's values reach is written
/// there, not in the view.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    /// The name views render it by, such as `blog/post.html`.
    pub name: String,
    pub path: std::path::PathBuf,
    /// Its unescaped lines, as `line: text`.
    pub unescaped: Vec<String>,
}

/// Unescaped lines shown per template, at most.
const UNESCAPED_LINES: usize = 8;
/// Templates shown with one view, at most.
pub const TEMPLATES: usize = 3;

/// The name a template under a `templates` directory is rendered by: the
/// path after the last `templates` part.
pub fn template_name(relative: &Path) -> Option<String> {
    let parts: Vec<&str> = relative.iter().filter_map(|p| p.to_str()).collect();
    let at = parts.iter().rposition(|p| *p == "templates")?;
    let extension = relative.extension().and_then(|e| e.to_str())?;
    (at + 1 < parts.len() && matches!(extension, "html" | "htm" | "txt" | "xml" | "jinja" | "j2"))
        .then(|| parts[at + 1..].join("/"))
}

/// The lines of a template that output values without escaping.
pub fn unescaped_lines(text: &str) -> Vec<String> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            compact.contains("|safe") || compact.contains("{%autoescapeoff%}")
        })
        .take(UNESCAPED_LINES)
        .map(|(index, line)| format!("{}: {}", index + 1, super::sites::clip(line.trim())))
        .collect()
}

/// The templates a function names in a string literal, such as
/// `render(request, 'blog/post.html', …)` or `template_name = "blog/post.html"`.
pub fn rendered<'t>(source: &str, templates: &'t [Template]) -> Vec<&'t Template> {
    templates
        .iter()
        .filter(|t| {
            source.contains(&format!("'{}'", t.name)) || source.contains(&format!("\"{}\"", t.name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(source: &str) -> tree_sitter::Tree {
        crate::syntax::parse(Path::new("a.py"), source)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn settings_modules_are_named_by_what_they_assign_and_where_they_live() {
        let project = "INSTALLED_APPS = ['app']\nDEBUG = True\n";
        let dev = "from .base import *\n\nDEBUG = True\nALLOWED_HOSTS = ['*']\n";
        let constants = "DEBUG = False\nLIMIT = 10\n";
        let check = |path: &str, source: &str| {
            settings_module(Path::new(path), tree(source).root_node(), source)
        };
        assert!(check("project/config.py", project));
        assert!(check("project/settings/dev.py", dev));
        assert!(check("project/settings.py", constants));
        assert!(
            !check("project/app/constants.py", constants),
            "a DEBUG flag elsewhere"
        );
        assert!(!check("project/settings/limits.py", "LIMIT = 10\n"));
        assert!(!check("project/settings.ts", project));
    }

    #[test]
    fn secret_literals_are_redacted_but_names_and_other_values_stay() {
        let source = "SECRET_KEY = 'abc123'\nDATABASES = {'default': {'PASSWORD': 'hunter2', 'NAME': 'db', 'USER': ''}}\nKEY = os.environ['KEY']\nDEBUG = True\n";
        let tree = tree(source);
        let mut found = Vec::new();
        secret_literals(tree.root_node(), source, &mut found);
        assert_eq!(found.len(), 2, "empty and non-literal values stay");
        assert_eq!(
            redacted(source, 0..source.len(), &found),
            "SECRET_KEY = '<redacted 6-character literal>'\nDATABASES = {'default': {'PASSWORD': '<redacted 7-character literal>', 'NAME': 'db', 'USER': ''}}\nKEY = os.environ['KEY']\nDEBUG = True\n"
        );
        let key = source.find("\nKEY =").unwrap() + 1;
        let line = key..key + source[key..].find('\n').unwrap();
        assert_eq!(
            redacted(source, line.clone(), &found),
            &source[line],
            "a statement without secrets is shown as written"
        );
    }

    #[test]
    fn setting_names_are_upper_case() {
        assert!(setting_name("SESSION_COOKIE_SECURE"));
        assert!(setting_name("X2"));
        assert!(!setting_name("debug"));
        assert!(!setting_name("_"));
        assert!(security_setting("SESSION_COOKIE_HTTPONLY"));
        assert!(!security_setting("LANGUAGE_CODE"));
        assert!(secret_name("AWS_SECRET_ACCESS_KEY"));
        assert!(!secret_name("LANGUAGE_CODE"));
    }

    #[test]
    fn django_code_is_python_that_imports_django_or_rest_framework() {
        let check = |path: &str, source: &str| {
            imports_django(Path::new(path), tree(source).root_node(), source)
        };
        assert!(check(
            "app/views.py",
            "from django.shortcuts import render\n"
        ));
        assert!(check(
            "app/api.py",
            "import rest_framework.views as views\n"
        ));
        assert!(check(
            "app/compat.py",
            "try:\n    from django.urls import path\nexcept ImportError:\n    path = None\n"
        ));
        assert!(!check(
            "app/main.py",
            "from fastapi import FastAPI\nimport djangoish\n"
        ));
        assert!(!check(
            "app/views.ts",
            "from django.shortcuts import render\n"
        ));
    }

    #[test]
    fn urlconf_routes_name_their_views_and_keep_the_pattern() {
        let source = "urlpatterns = [\n    path('orders/<int:order_id>/', views.order_detail, name='detail'),\n    re_path(r'^t/(?P<task_id>\\d+)/$',\n        views.task_edit),\n    path('edit/', OrderEditView.as_view(), name='edit'),\n    url(r'^$', 'shop.views.index'),\n    path('api/', include(router.urls)),\n]\nrouter.register(r'orders', api.OrderViewSet, basename='order')\n";
        let found = routes(tree(source).root_node(), source);
        let views: Vec<(Option<&str>, &str, usize)> = found
            .iter()
            .map(|r| (r.module.as_deref(), r.view.as_str(), r.line))
            .collect();
        assert_eq!(
            views,
            [
                (Some("views"), "order_detail", 2),
                (Some("views"), "task_edit", 3),
                (None, "OrderEditView", 5),
                (Some("views"), "index", 6),
                (Some("api"), "OrderViewSet", 9),
            ],
            "an include routes to no view"
        );
        assert_eq!(
            found[1].text, "re_path(r'^t/(?P<task_id>\\d+)/$', views.task_edit)",
            "a route spread over lines is shown on one"
        );
        let views_py = Path::new("shop/views.py");
        assert!(routes_to(&found[0], views_py, "order_detail", ""));
        assert!(!routes_to(
            &found[0],
            Path::new("shop/api.py"),
            "order_detail",
            ""
        ));
        assert!(routes_to(&found[2], views_py, "post", "OrderEditView"));
        assert!(!routes_to(
            &found[2],
            views_py,
            "clean_note",
            "OrderEditView"
        ));
        assert!(routes_to(
            &found[4],
            Path::new("shop/api/__init__.py"),
            "list",
            "OrderViewSet"
        ));
    }

    #[test]
    fn settings_extend_the_modules_they_star_import() {
        let production =
            "from .base import *  # noqa\nfrom config.settings.cors import *\nimport os\n";
        assert!(extends(production, Path::new("site/settings/base.py")));
        assert!(extends(production, Path::new("config/settings/cors.py")));
        assert!(!extends(production, Path::new("site/settings/os.py")));
        assert!(!extends(
            "from .base import Base\n",
            Path::new("site/settings/base.py")
        ));
    }

    #[test]
    fn management_commands_are_named_by_their_module() {
        let command = |path: &'static str| management_command(Path::new(path));
        assert_eq!(
            command("shop/orders/management/commands/export_orders.py"),
            Some("export_orders")
        );
        assert_eq!(command("shop/orders/management/commands/__init__.py"), None);
        assert_eq!(command("shop/orders/commands/export_orders.py"), None);
    }

    #[test]
    fn module_constants_are_shown_with_secrets_redacted() {
        let source = "import os\n\nINVOICE_DIR = os.path.join(BASE_DIR, 'invoices')\nAPI_TOKEN = 's3cr3t-value'\nlimit = 3\n\ndef f():\n    X = 1\n";
        assert_eq!(
            module_constants(tree(source).root_node(), source),
            [
                (
                    "INVOICE_DIR".to_string(),
                    "INVOICE_DIR = os.path.join(BASE_DIR, 'invoices')".to_string()
                ),
                (
                    "API_TOKEN".to_string(),
                    "API_TOKEN = '<redacted 12-character literal>'".to_string()
                ),
            ]
        );
    }

    #[test]
    fn a_dotted_path_under_a_secret_name_is_not_a_secret() {
        let source = "JWT_AUTH = {'JWT_GET_USER_SECRET_KEY': 'app.auth.services.user_secret_key', 'JWT_SECRET_KEY': 'k3y!'}\n";
        let tree = tree(source);
        let mut found = Vec::new();
        secret_literals(tree.root_node(), source, &mut found);
        assert_eq!(
            redacted(source, 0..source.len(), &found),
            "JWT_AUTH = {'JWT_GET_USER_SECRET_KEY': 'app.auth.services.user_secret_key', 'JWT_SECRET_KEY': '<redacted 4-character literal>'}\n"
        );
    }

    #[test]
    fn templates_are_named_after_their_templates_directory_and_list_unescaped_lines() {
        assert_eq!(
            template_name(Path::new("shop/orders/templates/orders/search.html")).as_deref(),
            Some("orders/search.html")
        );
        assert_eq!(template_name(Path::new("shop/templates.html")), None);
        assert_eq!(
            unescaped_lines(
                "<h1>{{ term }}</h1>\n<p>{{ term | safe }}</p>\n{% autoescape off %}{{ x }}{% endautoescape %}\n"
            ),
            [
                "2: <p>{{ term | safe }}</p>",
                "3: {% autoescape off %}{{ x }}{% endautoescape %}"
            ]
        );
        let templates = [Template {
            name: "orders/search.html".into(),
            path: "shop/orders/templates/orders/search.html".into(),
            unescaped: Vec::new(),
        }];
        assert_eq!(
            rendered(
                "return render(request, 'orders/search.html', {})",
                &templates
            )
            .len(),
            1
        );
        assert!(
            rendered(
                "return render(request, 'orders/search_safe.html', {})",
                &templates
            )
            .is_empty()
        );
    }

    #[test]
    fn selections_name_a_settings_module_by_its_dotted_path() {
        let dockerfile = selections_in(
            Path::new("Dockerfile"),
            "FROM python\nENV DJANGO_SETTINGS_MODULE=shop.settings.production\n",
        );
        let manage = selections_in(
            Path::new("manage.py"),
            "os.environ.setdefault('DJANGO_SETTINGS_MODULE', 'shop.settings.dev')\n",
        );
        let all: Vec<Selection> = dockerfile.into_iter().chain(manage).collect();
        let production = selected_by(Path::new("shop/settings/production.py"), &all);
        assert_eq!(production.len(), 1);
        assert_eq!(production[0].line, 2);
        assert!(selected_by(Path::new("shop/settings/prod.py"), &all).is_empty());
        assert!(!selection_file(Path::new(".env.production")));
        assert!(selection_file(Path::new("deploy/app.yaml")));
    }
}
