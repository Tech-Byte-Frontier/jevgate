//! Astro, Vue and Svelte files, parsed as their scripts: every byte outside
//! Astro's frontmatter and `<script>` contents becomes a space and newlines
//! are kept, so byte offsets and lines in the parsed tree are the file's.
//! Markup and styles are left out.
use std::ops::Range;

/// Single-file component formats whose scripts are parsed as TypeScript or
/// JavaScript.
pub const FORMATS: [&str; 3] = ["astro", "vue", "svelte"];

/// Script types that hold JavaScript or TypeScript.
const SCRIPT_TYPES: [&str; 4] = [
    "module",
    "text/javascript",
    "application/javascript",
    "text/typescript",
];

/// The file with only its scripts kept, and their grammar: TypeScript when a
/// script says `lang="ts"` (always for Astro, whose frontmatter and scripts
/// are TypeScript), TSX for `lang="tsx"`, else JavaScript.
pub fn scripts(extension: &str, source: &str) -> (String, tree_sitter::Language) {
    let astro = extension == "astro";
    let frontmatter = astro.then(|| frontmatter(source)).flatten();
    let from = frontmatter.as_ref().map_or(0, |r| r.end);
    let elements = script_elements(source, from);
    let keep: Vec<Range<usize>> = frontmatter.into_iter().chain(elements.ranges).collect();
    let language = if elements.tsx {
        tree_sitter_typescript::LANGUAGE_TSX
    } else if astro || elements.typescript {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT
    } else {
        tree_sitter_javascript::LANGUAGE
    };
    (masked(source, &keep), language.into())
}

/// Astro's frontmatter: the text between the leading `---` fences.
fn frontmatter(source: &str) -> Option<Range<usize>> {
    let start = source.len() - source.trim_start().len();
    if !source[start..].starts_with("---") {
        return None;
    }
    let body = start
        + source[start..]
            .find('\n')
            .map_or(source.len() - start, |i| i + 1);
    let end = source[body..]
        .match_indices("---")
        .map(|(i, _)| body + i)
        .find(|&i| source[..i].ends_with('\n'))
        .unwrap_or(source.len());
    Some(body.min(end)..end)
}

/// The contents of `<script>` elements after `from`, and their languages.
struct Elements {
    ranges: Vec<Range<usize>>,
    typescript: bool,
    tsx: bool,
}

fn script_elements(source: &str, mut from: usize) -> Elements {
    let lower = source.to_ascii_lowercase();
    let mut found = Elements {
        ranges: Vec::new(),
        typescript: false,
        tsx: false,
    };
    while let Some(open) = lower[from..].find("<script").map(|i| from + i) {
        let Some(tag_end) = lower[open..].find('>').map(|i| open + i + 1) else {
            break;
        };
        let tag = &lower[open..tag_end];
        let Some(close) = lower[tag_end..].find("</script").map(|i| tag_end + i) else {
            break;
        };
        if holds_script(tag) {
            found.typescript |= ["lang=\"ts\"", "lang='ts'", "lang=\"typescript\""]
                .iter()
                .any(|l| tag.contains(l));
            found.tsx |= tag.contains("lang=\"tsx\"") || tag.contains("lang='tsx'");
            found.ranges.push(tag_end..close);
        }
        from = close;
    }
    found
}

/// Whether an opening `<script …>` tag holds code here: not an external
/// source, and no type other than JavaScript or TypeScript.
fn holds_script(tag: &str) -> bool {
    !tag.contains(" src=")
        && !tag.ends_with("/>")
        && tag.split("type=").nth(1).is_none_or(|t| {
            let t = t.trim_start_matches(['"', '\'']);
            SCRIPT_TYPES.iter().any(|kind| t.starts_with(kind))
        })
}

/// `source` with every byte outside `keep` a space, newlines kept.
fn masked(source: &str, keep: &[Range<usize>]) -> String {
    let mut masked = String::with_capacity(source.len());
    let mut kept = keep.iter().peekable();
    for (i, c) in source.char_indices() {
        while kept.peek().is_some_and(|r| r.end <= i) {
            kept.next();
        }
        if c == '\n' || kept.peek().is_some_and(|r| r.contains(&i)) {
            masked.push(c);
        } else {
            masked.extend(std::iter::repeat_n(' ', c.len_utf8()));
        }
    }
    masked
}
