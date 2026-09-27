//! What runs before a test, read from its file's text: the setup head and hooks
//! of a test file, a Ruby test's groups, `let` and hooks, and the support file
//! nearest a test.
use super::*;

/// Setup text before the first test, and each setup hook, above these sizes
/// is left out rather than cut.
pub(super) const SETUP_BYTES: usize = 4000;
pub(super) const HOOK_BYTES: usize = 1500;

/// Test helpers shown with one Ruby case, at most.
pub(super) const HELPERS: usize = 4;

/// A Ruby case's setup: the file's head, the hooks its groups declare, then
/// the test helpers the case and its hooks call, and the helpers those call.
/// Also the other files the helpers come from.
pub(super) fn ruby_setup(
    file: &FileContext<'_>,
    case: &TestCase,
    head: Option<String>,
    helpers: &BTreeMap<String, Vec<SubjectSource>>,
) -> (String, Vec<PathBuf>) {
    let setup = case_setup(file.source, head, &case.hooks);
    let mut names: Vec<String> = case.calls.iter().chain(&case.hook_calls).cloned().collect();
    let mut shown: Vec<&SubjectSource> = Vec::new();
    let mut next = 0;
    while next < names.len() && shown.len() < HELPERS {
        let name = names[next].clone();
        next += 1;
        let Some(defined) = helpers.get(&name) else {
            continue;
        };
        let found = nearest(file.path, defined);
        let Some(helper) = found.filter(|h| {
            h.source.len() <= HOOK_BYTES
                && !setup.contains(h.source.as_str())
                && !shown.iter().any(|s| s.source == h.source)
        }) else {
            continue;
        };
        shown.push(helper);
        if let Some(tree) = crate::syntax::parse(&helper.path, &helper.source)
            .ok()
            .flatten()
        {
            let mut calls = Vec::new();
            crate::analysis::ruby::called_names(tree.root_node(), &helper.source, &mut calls);
            names.extend(calls);
        }
    }
    let mut parts: Vec<String> = (!setup.is_empty()).then_some(setup).into_iter().collect();
    parts.extend(shown.iter().map(|h| h.source.clone()));
    let paths = shown
        .iter()
        .filter(|h| h.path != file.path)
        .map(|h| h.path.clone())
        .collect();
    (parts.join("\n\n"), paths)
}

/// The one definition of a helper nearest the test: in its own file, else
/// in the support file that shares the most directories with it, at least
/// one, as `test/test_helper.rb` does with `test/routing_test.rb`. None when
/// two definitions are equally near.
pub(in crate::units) fn nearest<'a>(
    test: &Path,
    defined: &'a [SubjectSource],
) -> Option<&'a SubjectSource> {
    let folders = |path: &Path| -> Vec<String> {
        path.parent()
            .into_iter()
            .flat_map(Path::components)
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect()
    };
    let own = folders(test);
    let shared = |helper: &SubjectSource| {
        if helper.path == test {
            return usize::MAX;
        }
        own.iter()
            .zip(folders(&helper.path))
            .take_while(|(a, b)| **a == *b)
            .count()
    };
    // A support file in another tree, sharing no directory with the test,
    // serves other tests: `test/test_helper.rb` is not a spec's helper.
    let callable = || {
        defined
            .iter()
            .filter(|h| h.path == test || h.shared && shared(h) > 0)
    };
    let best = callable().map(shared).max()?;
    let mut nearest = callable().filter(|h| shared(h) == best);
    let first = nearest.next();
    nearest.next().is_none().then_some(first).flatten()
}

/// One case's setup: the file's head, then the hooks its groups declare. A
/// hook larger than its limit is left out rather than cut, and so are the
/// hooks when together they are too long.
pub(super) fn case_setup(source: &str, head: Option<String>, hooks: &[Range<usize>]) -> String {
    let kept = usize::from(head.is_some());
    let mut parts: Vec<String> = head.into_iter().collect();
    parts.extend(
        hooks
            .iter()
            .map(|hook| source[hook.clone()].to_string())
            .filter(|text| text.len() <= HOOK_BYTES),
    );
    let setup = parts.join("\n\n");
    if setup.len() <= SETUP_BYTES + HOOK_BYTES {
        setup
    } else {
        parts.truncate(kept);
        parts.join("")
    }
}

/// The hooks a case's groups declare, as sent beside a pair of tests.
pub(super) fn hook_text(source: &str, case: &TestCase) -> String {
    case_setup(source, None, &case.hooks)
}

/// A test file's shared setup: the text of its test region before the first
/// test or suite (imports, mocks, fixtures), then each setup hook. A part
/// larger than its limit is left out rather than cut.
pub(in crate::units) fn file_setup(source: &str, region_start: usize, first_case: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let start = region_start.saturating_sub(1).min(lines.len());
    let mut parts: Vec<String> = setup_head(&lines, start, first_case).into_iter().collect();
    parts.extend(setup_hooks(&lines[start..].join("\n")));
    let setup = parts.join("\n\n");
    if setup.len() <= SETUP_BYTES + HOOK_BYTES {
        setup
    } else {
        parts.truncate(1);
        parts.join("")
    }
}

/// The lines from `start` up to the first suite, test, test module or Java
/// setup method, when they are short enough to send. A Java test class's
/// fields, such as its mocks, are part of the head.
pub(super) fn setup_head(lines: &[&str], start: usize, first_case: usize) -> Option<String> {
    const OPENERS: &[&str] = &[
        "describe(",
        "describe.",
        "suite(",
        "context(",
        "test(",
        "test.",
        "it(",
        "it.",
        "def test",
        "class ",
        "mod tests",
        "#[test]",
        // Ruby: RSpec groups and examples, and Rails `test "…" do`.
        "describe ",
        "RSpec.describe",
        "context ",
        "it ",
        "test ",
        "module ",
        // Java: setup methods and `@Nested` test classes.
        "@Before",
        "@Nested",
    ];
    let last = first_case.saturating_sub(1).min(lines.len());
    let end = (start..last)
        .find(|&i| {
            let line = lines[i].trim_start();
            // A Python class opens a suite; a braced class holds the fields
            // the tests share.
            OPENERS.iter().any(|opener| line.starts_with(opener))
                && !(line.starts_with("class ") && line.trim_end().ends_with('{'))
        })
        .unwrap_or(last);
    let head = lines[start..end].join("\n");
    (!head.trim().is_empty() && head.len() <= SETUP_BYTES).then(|| head.trim().to_string())
}

/// Every setup hook in `region` short enough to send: `beforeEach`/`beforeAll`
/// calls, Python `setUp`/`setup_method` methods and Java methods annotated
/// `@BeforeEach`, `@BeforeAll`, `@Before` or `@BeforeClass`.
pub(super) fn setup_hooks(region: &str) -> Vec<String> {
    let mut hooks = Vec::new();
    for (at, _) in region.match_indices("@Before") {
        let name_end = at
            + region[at + 1..]
                .find(|c: char| !c.is_alphanumeric())
                .map_or(region.len() - at, |i| i + 1);
        if matches!(
            &region[at..name_end],
            "@BeforeEach" | "@BeforeAll" | "@Before" | "@BeforeClass"
        ) {
            let text = braced_method(&region[at..]);
            if text.len() <= HOOK_BYTES {
                hooks.push(text.to_string());
            }
        }
    }
    for hook in [
        "beforeEach(",
        "beforeAll(",
        "def setUp(",
        "def setup_method(",
    ] {
        let mut from = 0;
        while let Some(found) = region[from..].find(hook) {
            let at = from + found;
            let text = if hook.starts_with("def ") {
                let line_start = region[..at].rfind('\n').map_or(0, |i| i + 1);
                indented_block(&region[at..], at - line_start)
            } else {
                balanced_call(&region[at..])
            };
            if text.len() <= HOOK_BYTES {
                hooks.push(text.to_string());
            }
            from = at + hook.len();
        }
    }
    hooks
}

/// A Java method from its annotation through the brace that closes its body.
pub(super) fn braced_method(text: &str) -> &str {
    let Some(open) = text.find('{') else {
        return text;
    };
    let mut depth = 0usize;
    for (i, c) in text[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[..open + i + 1];
                }
            }
            _ => {}
        }
    }
    text
}

/// A call from its name through the parenthesis that closes it.
pub(super) fn balanced_call(text: &str) -> &str {
    let mut depth = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return &text[..i + 1];
                }
            }
            _ => {}
        }
    }
    text
}

/// A Python definition line, indented `base` columns, and the lines indented below it.
pub(super) fn indented_block(text: &str, base: usize) -> &str {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return text;
    };
    let mut end = first.len();
    let indent = |line: &str| line.len() - line.trim_start().len();
    for line in lines {
        if !line.trim().is_empty() && indent(line) <= base {
            break;
        }
        end += line.len();
    }
    text[..end].trim_end()
}
