//! Copies that are not compared: in example, benchmark or retired directories,
//! in code marked deprecated, and between Bend 2 tests pinned to their output.
use super::*;

/// Whether a copy lies in a function or type marked deprecated: it goes
/// with the next major version, so sharing its code with its replacement
/// is not worth doing. flysystem's deprecated phpseclib 2 adapter was
/// paired with its phpseclib 3 successor in 7 reviews.
pub(super) fn deprecated(files: &[SourceFile<'_>], site: &Site) -> bool {
    let file = &files[site.file];
    let Ok(Some(tree)) = crate::syntax::parse(file.path, file.source) else {
        return false;
    };
    let mut node = tree
        .root_node()
        .descendant_for_byte_range(site.span.start, site.span.start);
    while let Some(current) = node {
        let kind = current.kind();
        let declaration = kind.ends_with("_declaration")
            || kind.ends_with("_definition")
            || kind.ends_with("_item")
            || matches!(kind, "method" | "class" | "module" | "function");
        if declaration && crate::analysis::units::deprecated(current, file.source) {
            return true;
        }
        node = current.parent();
    }
    false
}

/// Whether a file lies in a directory of retired code, such as
/// `deprecated`, `archive` or a proof of concept: like code marked
/// deprecated, it is not worth sharing code with. A Unity project's
/// `Assets/ProofOfConcept` builders, kept as a reference with no menu entry,
/// were paired with the live scene builders in six wrong reviews. `legacy`
/// is left out, since legacy code is often still served.
pub(super) fn retired(path: &Path) -> bool {
    path.parent().is_some_and(|dir| {
        dir.iter().any(|part| {
            let part = part
                .to_string_lossy()
                .to_ascii_lowercase()
                .replace(['-', '_'], "");
            [
                "deprecated",
                "archive",
                "archived",
                "attic",
                "graveyard",
                "retired",
                "obsolete",
                "proofofconcept",
                "poc",
                "pocs",
            ]
            .contains(&part.as_str())
        })
    })
}

/// A directory of example code: `examples`, `demo`, `tutorial`, or a name
/// such as `blog_examples`.
pub(super) fn example_directory(part: &str) -> bool {
    let part = part.to_ascii_lowercase();
    [
        "example",
        "examples",
        "demo",
        "demos",
        "tutorial",
        "tutorials",
        "docs_src",
    ]
    .contains(&part.as_str())
        || part.ends_with("_examples")
        || part.ends_with("-examples")
}

/// Two Bend 2 tests: each is a whole program pinned to the output its run
/// prints, so their copies are the point of each test. Of 4 shared-logic
/// findings between such tests on thirteen Bend 2 projects, all were wrong.
pub(super) fn separate_tests(a: &SourceFile<'_>, b: &SourceFile<'_>) -> bool {
    let test = |f: &SourceFile<'_>| {
        crate::analysis::bend::file(f.path)
            && crate::analysis::bend::expected_output(f.source).is_some()
    };
    a.path != b.path && test(a) && test(b)
}

/// Whether a file sits in a benchmark directory.
pub(crate) fn benchmark_code(path: &Path) -> bool {
    path.parent().is_some_and(|dir| {
        dir.iter()
            .any(|part| benchmark_directory(&part.to_string_lossy()))
    })
}

pub(super) fn benchmark_directory(part: &str) -> bool {
    matches!(
        part.to_ascii_lowercase().as_str(),
        "bench" | "benches" | "benchmark" | "benchmarks"
    )
}

/// Whether a file is example code, written to be read beside other examples.
/// Also a top-level `samples` or `sample` directory (a Java package named
/// `samples` is source), a .NET project named like `MediatR.Examples.Autofac`,
/// and Go's `example_*_test.go` files, which show how to call a package.
/// Directories below a JVM source root (`src/main/java`) are packages, not
/// examples: Spring Initializr names a new project's package
/// `com.example.demo`, which made every finding of such a project a note.
pub(crate) fn example_code(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let top = path
        .iter()
        .next()
        .map(|p| p.to_string_lossy().to_ascii_lowercase())
        .filter(|_| path.iter().count() > 1)
        .unwrap_or_default();
    let directories: Vec<String> = path
        .parent()
        .map(|dir| {
            dir.iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    let packages = jvm_source_root(&directories).unwrap_or(directories.len());
    (name.starts_with("example_") && name.ends_with("_test.go"))
        || matches!(top.as_str(), "samples" | "sample")
        || directories[..packages]
            .iter()
            .any(|part| example_directory(part) || part.to_ascii_lowercase().contains(".examples"))
}

/// Where the package directories of a JVM source root begin: after
/// `src/<source set>/java` (or `kotlin`, `scala`, `groovy`).
pub(super) fn jvm_source_root(directories: &[String]) -> Option<usize> {
    directories
        .windows(3)
        .position(|w| {
            w[0] == "src" && matches!(w[2].as_str(), "java" | "kotlin" | "scala" | "groovy")
        })
        .map(|at| at + 3)
}

/// Whether two files are separate variants of one example, kept side by
/// side on purpose: under the same `examples` (or `demo`, `tutorial`)
/// directory, in different directories below it. django-styleguide shows a
/// Google login flow written by hand in `blog_examples/…/raw` and with the
/// SDK in `…/sdk`; their copies are the point. Benchmarks kept so are
/// separate programs too: each of bendlang/bend's `bench/runtime/*` is a
/// standalone program measured beside its C, TypeScript and Lean twins,
/// and the 6 shared-logic findings across them were labeled wrong.
pub(super) fn separate_examples(a: &Path, b: &Path) -> bool {
    let example = |part: &str| example_directory(part) || benchmark_directory(part);
    let dirs = |p: &Path| -> Vec<String> {
        p.parent()
            .map(|d| d.iter().map(|c| c.to_string_lossy().into_owned()).collect())
            .unwrap_or_default()
    };
    let (a, b) = (dirs(a), dirs(b));
    let Some(root) = a.iter().zip(&b).position(|(x, y)| x == y && example(x)) else {
        return false;
    };
    a[..=root] == b[..=root] && a[root + 1..] != b[root + 1..]
}
