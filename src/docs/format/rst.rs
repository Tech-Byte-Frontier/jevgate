//! reStructuredText read as Markdown: section titles become `#` headings by
//! the order their adornment styles appear, comments are dropped, and code
//! directives and literal blocks are fenced.

/// Directives whose content is code or data rather than prose.
const CODE_DIRECTIVES: &[&str] = &[
    "code-block",
    "code",
    "sourcecode",
    "parsed-literal",
    "math",
    "raw",
    "doctest",
    "testcode",
    "testoutput",
    "testsetup",
    "testcleanup",
    "ipython",
    "jupyter-execute",
    "csv-table",
    "graphviz",
    "digraph",
    "graph",
    "mermaid",
    "tabs-code",
];

/// Directives whose argument names a file of the repository.
const PATH_DIRECTIVES: &[&str] = &["include", "literalinclude", "image", "figure"];

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// The character of a line that is one punctuation character repeated.
fn adornment(line: &str) -> Option<char> {
    let line = line.trim_end();
    let first = line.chars().next()?;
    (line.len() >= 2 && first.is_ascii_punctuation() && line.chars().all(|c| c == first))
        .then_some(first)
}

/// The end (exclusive) of the lines indented past `column` from `start`,
/// without trailing blank lines.
fn indented_end(lines: &[&str], start: usize, column: usize) -> usize {
    let mut end = start;
    for (j, line) in lines.iter().enumerate().skip(start) {
        if blank(line) {
            continue;
        }
        if indent(line) <= column {
            break;
        }
        end = j + 1;
    }
    end
}

/// The name and argument of a directive, `name:: argument`; none for a
/// comment.
fn directive(rest: &str) -> Option<(&str, &str)> {
    rest.split_once("::")
        .map(|(name, argument)| (name.trim(), argument.trim()))
        .filter(|(name, _)| !name.is_empty() && !name.contains(char::is_whitespace))
}

/// The language a code directive's fence names, as Markdown's would.
fn fence_language<'a>(name: &'a str, argument: &'a str) -> &'a str {
    match name {
        "code-block" | "code" | "sourcecode" => argument.split_whitespace().next().unwrap_or(""),
        _ if name.starts_with("test")
            || ["doctest", "ipython", "jupyter-execute"].contains(&name) =>
        {
            "python"
        }
        _ => name,
    }
}

struct Rst<'a> {
    lines: &'a [&'a str],
    out: Vec<String>,
    code: Vec<bool>,
    /// Adornment styles in the order they first appear: character and
    /// whether the title is overlined.
    styles: Vec<(char, bool)>,
}

pub(super) fn rst(lines: &[&str]) -> Vec<String> {
    let mut doc = Rst {
        lines,
        out: lines.iter().map(|l| (*l).to_string()).collect(),
        code: vec![false; lines.len()],
        styles: Vec::new(),
    };
    let mut i = 0;
    while i < lines.len() {
        i = doc.line(i);
    }
    for (line, code) in doc.out.iter_mut().zip(&doc.code) {
        if !code {
            *line = line.replace("``", "`");
        }
    }
    doc.out
}

impl Rst<'_> {
    /// Read the construct starting at line `i`; returns the next line to read.
    fn line(&mut self, i: usize) -> usize {
        let lines = self.lines;
        let line = lines[i];
        if blank(line) {
            return i + 1;
        }
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("..")
            && (rest.is_empty() || rest.starts_with(' '))
        {
            return self.explicit(i, rest.trim());
        }
        if indent(line) == 0
            && let Some(next) = self.title(i)
        {
            return next;
        }
        if line.trim_end().ends_with("::") {
            let end = self.literal(i, indent(line));
            if end > i + 1 {
                return end;
            }
        }
        i + 1
    }

    /// A section title, overlined or not, or a transition.
    fn title(&mut self, i: usize) -> Option<usize> {
        let lines = self.lines;
        let line = lines[i];
        let after_blank = i == 0 || blank(lines[i - 1]);
        match adornment(line) {
            Some(c) => {
                let (Some(text), Some(under)) = (lines.get(i + 1), lines.get(i + 2)) else {
                    return (after_blank && lines.get(i + 1).is_none_or(|l| blank(l)))
                        .then(|| self.blank_line(i, i + 1));
                };
                if after_blank
                    && !blank(text)
                    && adornment(text).is_none()
                    && adornment(under) == Some(c)
                {
                    self.heading(i + 1, c, true);
                    self.out[i].clear();
                    self.out[i + 2].clear();
                    return Some(i + 3);
                }
                (after_blank && blank(text)).then(|| self.blank_line(i, i + 1))
            }
            None => {
                let under = lines.get(i + 1)?;
                let c = adornment(under)?;
                let width = line.trim().chars().count();
                let long_enough = under.trim_end().len() >= width.min(4);
                (after_blank && long_enough).then(|| {
                    self.heading(i, c, false);
                    self.out[i + 1].clear();
                    i + 2
                })
            }
        }
    }

    fn blank_line(&mut self, i: usize, next: usize) -> usize {
        self.out[i].clear();
        next
    }

    fn heading(&mut self, i: usize, c: char, overlined: bool) {
        let style = (c, overlined);
        let level = match self.styles.iter().position(|s| *s == style) {
            Some(p) => p + 1,
            None => {
                self.styles.push(style);
                self.styles.len()
            }
        }
        .min(crate::docs::markdown::DEEPEST_HEADING);
        self.out[i] = format!("{} {}", "#".repeat(level), self.lines[i].trim());
    }

    /// A comment, directive, target or substitution starting with `..`.
    fn explicit(&mut self, i: usize, rest: &str) -> usize {
        let column = indent(self.lines[i]);
        let end = indented_end(self.lines, i + 1, column);
        // Targets, footnotes, citations and substitutions stay as written.
        if rest.starts_with(['_', '[', '|']) {
            return i + 1;
        }
        let Some((name, argument)) = directive(rest) else {
            return self.comment(i, end);
        };
        if PATH_DIRECTIVES.contains(&name) && !argument.is_empty() && !argument.contains(' ') {
            let line = self.lines[i];
            let marker = &line[..line.find("::").unwrap_or(0) + 2];
            self.out[i] = format!("{marker} `{argument}`");
        }
        if CODE_DIRECTIVES.contains(&name) {
            return self.code_directive(i, end, column, fence_language(name, argument));
        }
        i + 1
    }

    /// A comment from `i` to `end`: every line of it is dropped.
    fn comment(&mut self, i: usize, end: usize) -> usize {
        let end = end.max(i + 1);
        for line in &mut self.out[i..end] {
            line.clear();
        }
        end
    }

    /// A code directive at `i` whose content ends at `end`: its options,
    /// which follow the directive line up to the first blank line, are
    /// dropped, and the code after them is fenced.
    fn code_directive(&mut self, i: usize, end: usize, column: usize, language: &str) -> usize {
        let lines = self.lines;
        let options = (i + 1..end)
            .take_while(|&j| !blank(lines[j]) && lines[j].trim_start().starts_with(':'))
            .count();
        for line in &mut self.out[i + 1..i + 1 + options] {
            line.clear();
        }
        self.fence(i + 1 + options, end, column, language);
        end.max(i + 1)
    }

    /// An indented literal block after a paragraph ending in `::`; returns
    /// the line after it, or `i + 1` when none follows.
    fn literal(&mut self, i: usize, column: usize) -> usize {
        let lines = self.lines;
        if lines.get(i + 1).is_none_or(|l| !blank(l)) {
            return i + 1;
        }
        let end = indented_end(lines, i + 1, column);
        if end <= i + 1 {
            return i + 1;
        }
        self.fence(i + 1, end, column, "");
        end
    }

    /// Fence the code in `start..end`: its first blank line opens the fence
    /// and the blank line after it closes it; without those lines, the code
    /// is indented past where a heading can start.
    fn fence(&mut self, start: usize, end: usize, column: usize, language: &str) {
        let lines = self.lines;
        let Some(open) = (start..end).find(|&j| blank(lines[j])) else {
            return;
        };
        for j in open..end {
            self.code[j] = true;
        }
        let closable = end == lines.len() || blank(lines[end]);
        if closable {
            let pad = " ".repeat(column);
            self.out[open] = format!("{pad}```{language}");
            if end < lines.len() {
                self.out[end] = format!("{pad}```");
                self.code[end] = true;
            }
        } else {
            for (out, line) in self.out[open..end].iter_mut().zip(&lines[open..end]) {
                if !blank(line) {
                    *out = format!("    {line}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::docs::{
        format::tests::{headings, view_lines, view_of},
        markdown,
    };

    #[test]
    fn rst_titles_become_headings_by_the_order_of_their_styles() {
        let source = "=====\nFlask\n=====\n\nIntro text.\n\nInstall\n-------\n\nRun it.\n\nPython Version\n~~~~~~~~~~~~~~\n\nText.\n\n----\n\nUsage\n-----\n\nShort\n--\n";
        assert_eq!(
            headings("docs/index.rst", source),
            [
                (2, 1, "Flask".to_string()),
                (7, 2, "Install".to_string()),
                (12, 3, "Python Version".to_string()),
                (19, 2, "Usage".to_string()),
            ]
        );
        let lines = view_lines("docs/index.rst", source, &[0, 2, 7, 16]);
        assert_eq!(
            (lines[21].as_str(), lines[22].as_str()),
            ("Short", "--"),
            "an underline shorter than 4 and its title is text"
        );
    }

    #[test]
    fn rst_comments_drop_and_code_is_fenced() {
        let source = "Setup\n=====\n\n.. This comment\n   spans lines.\n\nRun this::\n\n   # install\n   pip install flask\n\nThen:\n\n.. code-block:: python\n   :caption: app.py\n\n   # create the app\n   app = Flask(__name__)\n\n.. note::\n\n   Use ``flask run``, see :file:`src/app.py`.\n\n.. literalinclude:: ../examples/app.py\n   :lines: 1-4\n";
        let text = view_of("docs/setup.rst", source);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "# Setup");
        assert_eq!((lines[3], lines[4]), ("", ""));
        assert_eq!(
            &lines[6..11],
            [
                "Run this::",
                "```",
                "   # install",
                "   pip install flask",
                "```"
            ]
        );
        assert_eq!(lines[13], ".. code-block:: python");
        assert_eq!(lines[14], "");
        assert_eq!(
            &lines[15..19],
            [
                "```python",
                "   # create the app",
                "   app = Flask(__name__)",
                "```"
            ]
        );
        assert_eq!(lines[21], "   Use `flask run`, see :file:`src/app.py`.");
        assert_eq!(lines[23], ".. literalinclude:: `../examples/app.py`");
        assert_eq!(
            headings("docs/setup.rst", source),
            [(1, 1, "Setup".to_string())]
        );
        let sections = markdown::parse(&text).sections;
        assert_eq!(sections.len(), 1);
        assert!(!sections[0].text.contains("This comment"));
    }
}
