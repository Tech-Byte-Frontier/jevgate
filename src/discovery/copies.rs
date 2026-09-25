//! Files the project did not write by hand: build output (generated-code
//! headers, source map references, minified text) and copied libraries.
use super::file_extension;
use std::path::Path;

/// A third-party library copied into the repository: a release file named
/// with its version (`jquery-3.6.0.js`, `editor-6.0.1.bundle.js`), the readable
/// build beside a minified one of the same name (`vue.js` and `vue.min.js`), or
/// a leading preserved license banner (`/*!` or `@license`) that names a
/// version. `path` is on disk, for the sibling.
pub fn vendored(path: &Path, source: Option<&str>) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let script = ["js", "mjs", "cjs"].contains(&file_extension(path).as_str());
    let stem = name.split('.').next().unwrap_or("");
    let minified_sibling = script
        && !name.contains(".min.")
        && path
            .with_file_name(format!("{stem}.min.{}", file_extension(path)))
            .is_file();
    (script && versioned_name(&name))
        || minified_sibling
        || shadcn(path)
        || source.is_some_and(license_banner)
        || script && asset(path) && source.is_some_and(license_text)
}

/// A component the shadcn CLI copied in: under a `shadcn` directory, or
/// under `ui` in a project whose `components.json` names ui.shadcn.com.
/// makerkit's `packages/ui/src/shadcn` says never to edit these files.
fn shadcn(path: &Path) -> bool {
    let component =
        ["tsx", "ts", "jsx", "js", "vue", "svelte"].contains(&file_extension(path).as_str());
    if !component {
        return false;
    }
    if path.iter().any(|part| part == "shadcn") {
        return true;
    }
    let Some(dir) = path
        .parent()
        .filter(|d| d.file_name().is_some_and(|n| n == "ui"))
    else {
        return false;
    };
    dir.ancestors().skip(1).take(4).any(|ancestor| {
        std::fs::read_to_string(ancestor.join("components.json"))
            .is_ok_and(|text| text.contains("ui.shadcn.com"))
    })
}

/// A path under a directory that serves static files or holds copied code.
fn asset(path: &Path) -> bool {
    path.iter().any(|part| {
        let part = part.to_string_lossy().to_lowercase();
        [
            "assets",
            "static",
            "public",
            "vendor",
            "vendors",
            "third_party",
            "third-party",
        ]
        .contains(&part.as_str())
    })
}

/// A leading comment that holds a whole license with its copyright, as a
/// library copied into a site's scripts keeps it: wtf's
/// `http/assets/scripts/reconnecting-websocket.js` opens with MIT's text.
fn license_text(source: &str) -> bool {
    let head: String = source
        .lines()
        .take(HEADER_LINES)
        .map(str::trim)
        .take_while(|line| line.is_empty() || COMMENT_STARTS.iter().any(|c| line.starts_with(c)))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    head.contains("copyright")
        && [
            "permission is hereby granted",
            "licensed under the apache license",
            "redistribution and use in source and binary forms",
        ]
        .iter()
        .any(|grant| head.contains(grant))
}

/// A file name that carries a release version, such as `lib-1.2.3.min.js`.
fn versioned_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..bytes.len()).any(|i| {
        matches!(bytes[i - 1], b'-' | b'.' | b'_')
            && name[i..]
                .trim_start_matches('v')
                .split('.')
                .take(3)
                .filter(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .count()
                == 3
    })
}

/// The first comment preserves a library's license and names its version:
/// bundlers keep `/*!` and `@license` comments in the builds they ship.
fn license_banner(source: &str) -> bool {
    let head = source.trim_start();
    let Some(end) = head.strip_prefix("/*").and_then(|rest| rest.find("*/")) else {
        return false;
    };
    let comment = &head[..end + 2];
    (comment.starts_with("/*!") || comment.contains("@license"))
        && comment
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .any(|word| {
                let parts: Vec<&str> = word.trim_matches('.').split('.').collect();
                parts.len() >= 3 && parts.iter().all(|p| !p.is_empty())
            })
}

/// A generated-code marker in the leading comment lines, such as `@generated`,
/// Go's `Code generated ... DO NOT EDIT.` or "automatically generated".
pub fn generated_header(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(HEADER_LINES)
        .take_while(|line| COMMENT_STARTS.iter().any(|c| line.starts_with(c)))
        .any(|line| {
            let line = line.to_ascii_lowercase();
            GENERATED_MARKERS.iter().any(|marker| line.contains(marker))
        })
}

/// Output of a bundler, minifier or compiler rather than source a person
/// edits: a generated-code header, a trailing source map reference, or text
/// whose lines are almost all longer than people write them.
pub fn generated_source(source: &str) -> bool {
    generated_header(source) || source_map_reference(source) || minified(source)
}

/// Compilers and bundlers end their output with `//# sourceMappingURL=…`.
fn source_map_reference(source: &str) -> bool {
    source
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .is_some_and(|line| {
            line.starts_with("//# sourceMappingURL=") || line.starts_with("/*# sourceMappingURL=")
        })
}

/// A line at least this long is not one a person wrote.
const MINIFIED_LINE_BYTES: usize = 1000;

/// Nine tenths of a file of at least `MINIFIED_LINE_BYTES` in lines of that
/// length: minified bundles are one or a few such lines. A long data line in
/// hand-written source leaves the rest of the file below that share.
fn minified(source: &str) -> bool {
    let long: usize = source
        .lines()
        .filter(|line| line.len() >= MINIFIED_LINE_BYTES)
        .map(str::len)
        .sum();
    source.len() >= MINIFIED_LINE_BYTES && long * 10 >= source.len() * 9
}

const COMMENT_STARTS: &[&str] = &["//", "#", "/*", "*", "<!--", "--", "\"\"\""];
/// Leading comment lines read for a marker: API client generators put theirs
/// after a title and description block.
const HEADER_LINES: usize = 30;
const GENERATED_MARKERS: &[&str] = &[
    "@generated",
    "do not edit",
    "automatically generated",
    "auto generated",
    "auto-generated",
    "autogenerated",
    "code generated by",
    "file was generated",
    "file is generated",
];
