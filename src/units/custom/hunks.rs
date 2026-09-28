//! The changed hunks of a file since `--base`, read from Git, for custom
//! questions about what a change adds or alters. A hunk works in any
//! language: it needs a diff, not a parser.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Diff lines one hunk unit holds at most: a longer hunk, such as a new
/// file, is asked about in parts, each well inside one request.
const MAX_LINES: usize = 80;

/// One changed hunk, or part of one.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Hunk {
    /// The first and last changed lines in the file as it is now; removed
    /// lines count at the line that now follows them.
    pub start: usize,
    pub end: usize,
    /// Its lines as Git shows them: `+` added, `-` removed, ` ` context.
    pub diff: String,
    /// The definition Git names in the hunk header, such as `fn charge(…)`.
    pub context: Option<String>,
    /// Only its added and removed lines, which identify it wherever it moves.
    pub changed: String,
}

impl Hunk {
    /// Its changed lines as a reader finds them: `40–52`, or `40`.
    pub fn lines(&self) -> String {
        if self.start == self.end {
            self.start.to_string()
        } else {
            format!("{}–{}", self.start, self.end)
        }
    }
}

/// The files changed since the base and where each was before.
pub(super) struct Changes {
    root: PathBuf,
    /// What a diff compares: the base and the working tree, or the agent
    /// hook's snapshots of the turn's start and of now.
    sides: Vec<String>,
    /// Current path to its previous path: `None` for a new or untracked file.
    paths: BTreeMap<PathBuf, Option<PathBuf>>,
}

impl Changes {
    /// The changes the check reviews (`revision::Changes::of_check`); none
    /// without a base, or when Git cannot list them, in which case the
    /// check's own selection of changed files has already failed.
    pub(super) fn load(root: &Path, args: &crate::options::CheckArgs) -> Option<Self> {
        let changes = crate::revision::Changes::of_check(root, args)?.ok()?;
        Some(Self {
            root: root.to_path_buf(),
            sides: changes.sides().into_iter().map(str::to_string).collect(),
            paths: changes.paths,
        })
    }

    /// The hunks of `path`, whose text is now `source`: none when it did
    /// not change, and all of it when Git does not track it yet. External
    /// diff and text conversion helpers never run, and a blank line of
    /// context keeps its space whatever `diff.suppressBlankEmpty` says.
    pub(super) fn hunks(&self, path: &Path, source: &str) -> Vec<Hunk> {
        let Some(previous) = self.paths.get(path) else {
            return Vec::new();
        };
        let mut args = vec![
            "-c",
            "diff.suppressBlankEmpty=false",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--find-renames",
            "--no-color",
            "--unified=3",
        ];
        args.extend(self.sides.iter().map(String::as_str));
        args.push("--");
        // A rename is a diff between both paths.
        let before = previous.as_deref().filter(|p| *p != path);
        args.extend(before.and_then(Path::to_str));
        let Some(now) = path.to_str() else {
            return Vec::new();
        };
        args.push(now);
        let diff = crate::revision::git(&self.root, &args)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let hunks = parse(&diff, source.lines().count());
        if hunks.is_empty() && previous.is_none() {
            added(source)
        } else {
            hunks
        }
    }
}

/// The hunks of one file's unified diff, parts of at most `MAX_LINES`
/// lines each; `lines` is how many lines the file has now. An empty line
/// inside a hunk is a blank line of context, as Git prints one when
/// `diff.suppressBlankEmpty` is set.
pub(super) fn parse(diff: &str, lines: usize) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut open: Option<(usize, Option<String>, Vec<&str>)> = None;
    for line in diff.lines() {
        if let Some(header) = line.strip_prefix("@@ ") {
            if let Some(hunk) = open.take() {
                split(hunk, lines, &mut hunks);
            }
            open = header_start(header).map(|(start, context)| (start, context, Vec::new()));
        } else if let Some((_, _, body)) = open.as_mut() {
            match line.as_bytes().first() {
                Some(b' ' | b'+' | b'-') => body.push(line),
                None => body.push(" "),
                // "\ No newline at end of file"
                Some(b'\\') => {}
                _ => split(open.take().unwrap(), lines, &mut hunks),
            }
        }
    }
    if let Some(hunk) = open {
        split(hunk, lines, &mut hunks);
    }
    hunks
}

/// The hunks of an example change written by hand: diff lines under `@@`
/// headers, or with none, one change from line 1. An empty line is an
/// unchanged blank line, since editors strip the space a diff gives it.
pub(super) fn example(diff: &str) -> Vec<Hunk> {
    let headed = diff.lines().any(|line| line.starts_with("@@ "));
    let header = if headed { "" } else { "@@ -1 +1 @@\n" };
    // No line count to keep within: the example is all the file there is.
    parse(&format!("{header}{diff}"), usize::MAX)
}

/// A new file's text as hunks that add every line.
fn added(source: &str) -> Vec<Hunk> {
    let body: Vec<String> = source.lines().map(|line| format!("+{line}")).collect();
    let body: Vec<&str> = body.iter().map(String::as_str).collect();
    let mut hunks = Vec::new();
    split((1, None, body), source.lines().count(), &mut hunks);
    hunks
}

/// The new file's first line and the definition a hunk header names:
/// `-12,7 +14,9 @@ fn charge()` is line 14 in `fn charge()`.
fn header_start(header: &str) -> Option<(usize, Option<String>)> {
    let mut parts = header.splitn(3, ' ');
    let _old = parts.next()?;
    let new = parts.next()?.strip_prefix('+')?;
    let start = new.split(',').next()?.parse().ok()?;
    let context = parts
        .next()
        .and_then(|rest| rest.strip_prefix("@@"))
        .map(str::trim)
        .filter(|context| !context.is_empty())
        .map(str::to_string);
    Some((start, context))
}

/// A hunk's body in parts of at most `MAX_LINES` lines, each with the lines
/// it changes; a part holding only context is left out.
fn split(
    (start, context, body): (usize, Option<String>, Vec<&str>),
    lines: usize,
    hunks: &mut Vec<Hunk>,
) {
    let last_line = lines.max(1);
    // The line of the file as it is now that the next context or added line
    // is, and where a removed line was.
    let mut next = start.max(1);
    for part in body.chunks(MAX_LINES) {
        let mut changed = Vec::new();
        let mut range: Option<(usize, usize)> = None;
        for line in part {
            let kind = line.as_bytes()[0];
            if kind == b'+' || kind == b'-' {
                range = Some(range.map_or((next, next), |(first, last)| (first, last.max(next))));
                changed.push(*line);
            }
            if kind != b'-' {
                next += 1;
            }
        }
        let Some((first, last)) = range else {
            continue;
        };
        hunks.push(Hunk {
            start: first.min(last_line),
            end: last.min(last_line),
            diff: part.join("\n"),
            context: context.clone(),
            changed: changed.join("\n"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_diff_splits_into_hunks_at_the_lines_they_change() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -10,4 +10,5 @@ fn charge(order: &Order) {\n     let total = order.total();\n-    log(total);\n+    log(order.body());\n+    audit(total);\n     send(total);\n@@ -40,2 +41,1 @@\n keep();\n-drop();\n\\ No newline at end of file\n";
        let hunks = parse(diff, 60);
        assert_eq!(hunks.len(), 2);
        assert_eq!((hunks[0].start, hunks[0].end), (11, 12));
        assert_eq!(
            hunks[0].context.as_deref(),
            Some("fn charge(order: &Order) {")
        );
        assert_eq!(
            hunks[0].changed,
            "-    log(total);\n+    log(order.body());\n+    audit(total);"
        );
        assert!(hunks[0].diff.starts_with("     let total"));
        assert_eq!(
            (hunks[1].start, hunks[1].end, hunks[1].lines()),
            (42, 42, "42".to_string()),
            "a removal counts at the line that now follows it"
        );
        assert_eq!(parse(diff, 41)[1].start, 41, "at most the file's last line");
        assert!(parse("Binary files a/x.png and b/x.png differ\n", 1).is_empty());
    }

    #[test]
    fn a_blank_line_of_context_printed_empty_stays_in_its_hunk() {
        // As Git prints it with `diff.suppressBlankEmpty` set.
        let diff = "@@ -1,5 +1,5 @@ def charge(order):\n def charge(order):\n-    x = 1\n+    x = 2\n\n-    y = 2\n+    y = log(order.body)\n     return x + y\n";
        let hunks = parse(diff, 5);
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].start, hunks[0].end), (2, 4));
        assert!(hunks[0].changed.ends_with("+    y = log(order.body)"));
        assert!(hunks[0].diff.contains("+    x = 2\n \n-    y = 2"));
    }

    #[test]
    fn a_long_hunk_or_new_file_is_asked_in_parts_without_context_only_parts() {
        let source: String = (1..=170).map(|n| format!("line {n}\n")).collect();
        let parts = added(&source);
        assert_eq!(parts.len(), 3);
        assert_eq!((parts[0].start, parts[0].end), (1, 80));
        assert_eq!(
            (parts[2].start, parts[2].end, parts[2].lines()),
            (161, 170, "161–170".into())
        );
        let mut body = vec!["+new"];
        body.extend(std::iter::repeat_n(" same", MAX_LINES));
        let mut hunks = Vec::new();
        split((5, None, body), 200, &mut hunks);
        assert_eq!(hunks.len(), 1, "the part of context alone is left out");
        assert_eq!((hunks[0].start, hunks[0].end), (5, 5));
    }
}
