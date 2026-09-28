//! The files a script reads in (`source`, `.`), for a language whose query
//! names them (`@include`): a Bash script shares code with another only
//! through them.
use super::{read, tags};
use std::{collections::BTreeSet, path::Path};

/// The names of the files a script reads in (`source lib/common.sh` reads
/// `common.sh`), for a language whose files share code only that way: its
/// query names them (`@include`), as Bash's does. None for the others.
pub(crate) fn includes(path: &Path, source: &str) -> Option<BTreeSet<String>> {
    let language = read(path, source)?;
    if !language.query().capture_names().contains(&"include") {
        return None;
    }
    let Ok(Some(tree)) = crate::syntax::parse(path, source) else {
        return Some(BTreeSet::new());
    };
    let found = tags(language, tree.root_node(), source).includes;
    Some(
        found
            .into_iter()
            .filter_map(|node| file_name(crate::analysis::text(node, source), source))
            .collect(),
    )
}

/// The file name at the end of a path as a script writes it:
/// `"$(dirname "$0")/lib.sh"` names `lib.sh`, and `"${apifile}"` the one
/// named where the script assigns `apifile=`, as pi-hole's scripts read
/// their helpers in.
fn file_name(written: &str, source: &str) -> Option<String> {
    let name = last_segment(written)?;
    match variable(name) {
        Some(variable) => assigned(variable, source).and_then(last_segment),
        None => Some(name),
    }
    .map(str::to_string)
}

fn last_segment(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next()?.trim_matches(['"', '\'', ' ']);
    (!name.is_empty()).then_some(name)
}

/// The variable a path is, whole: `${apifile}` or `$apifile`.
fn variable(name: &str) -> Option<&str> {
    let inner = name.strip_prefix('$')?;
    let inner = inner
        .strip_prefix('{')
        .and_then(|i| i.strip_suffix('}'))
        .unwrap_or(inner);
    inner
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
        .then_some(inner)
}

/// The value a script assigns to `variable` at the start of a line.
fn assigned<'s>(variable: &str, source: &'s str) -> Option<&'s str> {
    source.lines().find_map(|line| {
        let line = ["local ", "readonly ", "declare ", "export "]
            .iter()
            .fold(line.trim_start(), |l, keyword| {
                l.strip_prefix(keyword).unwrap_or(l)
            });
        line.strip_prefix(variable)?.strip_prefix('=')
    })
}
