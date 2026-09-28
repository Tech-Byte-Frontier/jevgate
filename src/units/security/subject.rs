//! What a security unit is judged on: a function, a file's module-level setup or
//! its template code, with the constants, types and enums its code names.
use super::*;

/// A function or setup statements that the security rules judge.
pub(in crate::units) struct Subject<'a> {
    /// Its position among its file's parsed units, which orders the function
    /// packs every rule shares; code outside them comes after them all.
    pub position: usize,
    pub name: String,
    /// `function` or `module`: the state key and the source path.
    pub kind: &'static str,
    pub source: String,
    pub sites: &'a [Site],
    /// Errors it creates, with their message arguments.
    pub errors: &'a [CreatedError],
    pub lines: (usize, usize),
    /// Functions that call it, as (name, source), for the injection recheck.
    pub callers: Vec<(String, String)>,
    /// Enums its sites name, such as `ConfigKey` in `'${ConfigKey.aiTag}'`,
    /// defined in this or another selected file: fixed choices, not
    /// parameters, which the trace otherwise could not tell apart.
    pub enums: Vec<String>,
    /// Definitions of the project's types its parameters name, shown when a
    /// path finding is confirmed: how a route parameter of that type is
    /// parsed decides what it can hold.
    pub types: Vec<String>,
    /// C# constants it names, as `Class.Field = value`: a key written in
    /// the code or a value read from configuration.
    pub constants: Vec<String>,
    /// Framework facts shown beside the source in every request of its
    /// units, such as the settings modules that import a settings module
    /// and assign its settings again.
    pub evidence: serde_json::Map<String, Value>,
    /// Whether it is Django code, whose questions name Django's calls and
    /// settings and ask its extra checks.
    pub django: bool,
    /// Errors the functions it calls create, with their messages, shown in
    /// its sensitive-data trace: whether an error's text that it sends is
    /// the program's own depends on where the error was raised.
    pub callee_errors: Vec<Value>,
    /// Whether its file sits at a test path, such as a test app's settings.
    pub test_path: bool,
}

/// The evidence key of the templates a function renders that write values
/// without escaping.
pub(in crate::units) const RENDERED: &str =
    "templates_it_renders_that_write_values_without_escaping";

impl Subject<'_> {
    pub(super) fn code(&self) -> String {
        format!("{}.source", self.kind)
    }

    /// Whether it renders a template that writes values without escaping,
    /// outside Django, whose questions name its templates already.
    pub(super) fn renders(&self) -> bool {
        !self.django && self.evidence.contains_key(RENDERED)
    }

    /// Its name, source and framework evidence, as sent.
    pub(super) fn state(&self) -> Value {
        let mut state = serde_json::Map::new();
        state.insert("name".into(), json!(self.name));
        state.insert("source".into(), json!(self.source));
        state.extend(self.evidence.clone());
        Value::Object(state)
    }
}

/// The subject of the unit at `position` among its file's parsed units.
pub(in crate::units) fn function_subject<'a>(
    file: &FileContext<'_>,
    (position, unit): (usize, &'a Unit),
    callers: Vec<(String, String)>,
    (enums, types): (&BTreeMap<String, String>, &BTreeMap<String, String>),
    constants: &BTreeMap<String, Vec<String>>,
) -> Subject<'a> {
    let source = unit.source(file.source).to_string();
    Subject {
        position,
        name: unit.name.clone(),
        kind: "function",
        constants: named_constants(file, &source, constants),
        source,
        sites: &unit.sites,
        errors: &unit.errors,
        lines: (unit.line, unit.end_line),
        callers,
        enums: named_enums(&unit.sites, enums),
        types: named_types(&unit.signature, types),
        evidence: serde_json::Map::new(),
        django: false,
        callee_errors: Vec::new(),
        test_path: false,
    }
}

/// Constant declarations shown with one subject, at most.
pub(super) const CONSTANTS: usize = 4;

/// The declarations of the C# constants a C# subject names.
pub(super) fn named_constants(
    file: &FileContext<'_>,
    source: &str,
    constants: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    if file.language != questions::CSHARP {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (field, declarations) in constants {
        let named = source.match_indices(field.as_str()).any(|(at, _)| {
            let before = source[..at].chars().next_back();
            let after = source[at + field.len()..].chars().next();
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            !word(before) && !word(after)
        });
        if named {
            found.extend(declarations.iter().cloned());
        }
    }
    found.truncate(CONSTANTS);
    found
}

/// Type definitions shown with one subject, at most.
pub(super) const TYPES: usize = 3;

/// The definitions of the project's types named as words in a signature.
pub(super) fn named_types(signature: &str, types: &BTreeMap<String, String>) -> Vec<String> {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    types
        .iter()
        .filter(|(name, _)| {
            signature.match_indices(name.as_str()).any(|(at, _)| {
                !word(signature[..at].chars().next_back())
                    && !word(signature[at + name.len()..].chars().next())
            })
        })
        .map(|(_, definition)| definition.clone())
        .take(TYPES)
        .collect()
}

/// Enum definitions shown with one subject, at most.
pub(super) const ENUMS: usize = 3;

/// The definitions of enums named as `Name.member` or `Name::member` in the sites.
pub(super) fn named_enums(sites: &[Site], enums: &BTreeMap<String, String>) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for site in sites {
        for (name, definition) in enums {
            let named = site.text.match_indices(name.as_str()).any(|(at, _)| {
                let before = site.text[..at].chars().next_back();
                let after = &site.text[at + name.len()..];
                before.is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
                    && (after.starts_with('.') || after.starts_with("::"))
            });
            if named && found.len() < ENUMS && !found.contains(definition) {
                found.push(definition.clone());
            }
        }
    }
    found
}

pub(in crate::units) fn setup_subject<'a>(
    file: &FileContext<'_>,
    setup: &'a crate::analysis::sites::Setup,
    constants: &BTreeMap<String, Vec<String>>,
) -> Option<Subject<'a>> {
    let first = setup.statements.first()?;
    let last = setup.statements.last()?;
    let source: Vec<String> = setup
        .statements
        .iter()
        .map(|(range, ..)| setup.text(file.source, range.clone()))
        .collect();
    let source = source.join("\n");
    let (name, kind) = if setup.script {
        (SCRIPT, "function")
    } else if setup.settings {
        (SETTINGS_MODULE, "module")
    } else {
        (MODULE_SETUP, "module")
    };
    Some(Subject {
        position: AFTER_UNITS,
        name: name.into(),
        kind,
        constants: named_constants(file, &source, constants),
        source,
        sites: &setup.sites,
        errors: &[],
        lines: (first.1, last.2),
        callers: Vec::new(),
        enums: Vec::new(),
        types: Vec::new(),
        evidence: serde_json::Map::new(),
        django: false,
        callee_errors: Vec::new(),
        test_path: false,
    })
}

/// A server template's code that reads client data, judged like a function
/// by every security rule.
pub(in crate::units) fn template_subject<'a>(
    file: &FileContext<'_>,
    code: &'a crate::analysis::sites::Setup,
) -> Option<Subject<'a>> {
    let first = code.statements.first()?;
    let last = code.statements.last()?;
    let source: Vec<&str> = code
        .statements
        .iter()
        .map(|(range, ..)| &file.source[range.clone()])
        .collect();
    Some(Subject {
        position: AFTER_UNITS,
        name: TEMPLATE_CODE.into(),
        kind: "function",
        source: source.join("\n"),
        sites: &code.sites,
        errors: &[],
        lines: (first.1, last.2),
        callers: Vec::new(),
        enums: Vec::new(),
        types: Vec::new(),
        constants: Vec::new(),
        evidence: serde_json::Map::new(),
        django: false,
        callee_errors: Vec::new(),
        test_path: false,
    })
}

/// The position of code outside the file's parsed units, such as a PHP page
/// script: security packed it after the functions.
const AFTER_UNITS: usize = usize::MAX;

/// The name of the unit that holds a server template's code.
pub(in crate::units) const TEMPLATE_CODE: &str = "template code";

/// The name of the unit that holds a file's top-level setup statements.
pub(in crate::units) const MODULE_SETUP: &str = "module setup";
/// The name of that unit in a Django settings module, whose statements
/// assign the deployed site's settings.
pub(in crate::units) const SETTINGS_MODULE: &str = "settings module";
/// The name of the unit that holds a PHP file's top-level statements.
pub(in crate::units) const SCRIPT: &str = "top-level code";
