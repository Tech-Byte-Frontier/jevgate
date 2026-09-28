use super::*;

/// Candidate pairs between two selected files, each a (path, source).
fn pairs_between(a: (&str, &str), b: (&str, &str)) -> usize {
    run(&[(a.0, a.1, true), (b.0, b.1, true)]).pairs.len()
}

/// The renamed names and values of the one pair between two selected files.
fn differences_between(a: (&str, &str), b: (&str, &str)) -> Vec<Difference> {
    let found = run(&[(a.0, a.1, true), (b.0, b.1, true)]);
    assert_eq!(found.pairs.len(), 1);
    found.pairs[0].differences.clone()
}

fn run(files: &[(&str, &str, bool)]) -> Candidates {
    let units: Vec<_> = files
        .iter()
        .map(|(path, source, _)| super::super::units::parse(Path::new(path), source).unwrap())
        .collect();
    let sources: Vec<_> = files
        .iter()
        .zip(&units)
        .map(|((path, source, selected), units)| SourceFile {
            path: Path::new(path),
            source,
            selected: *selected,
            units: &units.units,
            excluded: Vec::new(),
            package: None,
        })
        .collect();
    find(&sources)
}

#[test]
fn copies_between_two_bend_tests_are_not_candidates() {
    let program = |output: &str| {
        format!(
            "import Base\n\ndef main() -> IO(Unit):\n  do IO<Unit>:\n    a : String <- IO.try(String, IO.get_env(\"HOME\"))\n    b : String <- IO.try(String, IO.get_env(\"USER\"))\n    c : String <- IO.try(String, IO.get_env(\"SHELL\"))\n    IO.print(a ++ b ++ c)\n{output}"
        )
    };
    let (golden, other) = (program("\n#|ok\n"), program(""));
    assert_eq!(
        pairs_between(("tests/io/a.bend", &golden), ("tests/io/b.bend", &golden)),
        0
    );
    assert_eq!(
        pairs_between(("tests/io/a.bend", &other), ("tests/io/b.bend", &other)),
        1,
        "tests that check themselves may share a helper"
    );
}

#[test]
fn copies_in_unrelated_packages_are_not_candidates() {
    let package = |dir: &str, dependencies: &[&str]| crate::packages::Package {
        dir: dir.into(),
        name: Some(dir.into()),
        dependencies: dependencies.iter().map(|d| d.to_string()).collect(),
    };
    let (a, b, shared) = (
        package("a", &["shared"]),
        package("b", &["shared"]),
        package("shared", &[]),
    );
    let units = super::super::units::parse(Path::new("x.rs"), LOAD).unwrap();
    let file = |path: &'static str, package| SourceFile {
        path: Path::new(path),
        source: LOAD,
        selected: true,
        units: &units.units,
        excluded: Vec::new(),
        package,
    };
    let separate = package("c", &[]);
    assert!(
        find(&[file("a/x.rs", Some(&a)), file("c/x.rs", Some(&separate))])
            .pairs
            .is_empty()
    );
    let linked = find(&[
        file("a/x.rs", Some(&a)),
        file("b/x.rs", Some(&b)),
        file("shared/x.rs", Some(&shared)),
    ]);
    assert!(!linked.pairs.is_empty());
}

const LOAD: &str = "fn load_user(path: &str) -> Result<User> {\n    let text = std::fs::read_to_string(path)?;\n    let value: Value = serde_json::from_str(&text)?;\n    let name = value[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    Ok(User { name })\n}\n";

#[test]
fn copies_in_deprecated_code_are_not_candidates() {
    assert_eq!(
        run(&[("a.rs", LOAD, true), ("b.rs", LOAD, true)])
            .pairs
            .len(),
        1
    );
    for mark in [
        "#[deprecated(note = \"use load_account\")]\n",
        "/// Deprecated: use load_account.\n",
        "/** @deprecated use load_account */\n",
    ] {
        let old = format!("{mark}{LOAD}");
        assert!(
            run(&[("a.rs", LOAD, true), ("b.rs", &old, true)])
                .pairs
                .is_empty(),
            "{mark}"
        );
    }
    // A method of a class whose documentation marks it deprecated.
    let class = |doc: &str| {
        format!(
            "<?php\n{doc}class Adapter\n{{\n    public function read(string $path): string\n    {{\n        $location = $this->prefix->prefixPath($path);\n        $contents = $this->connection->get($location);\n        if ($contents === false) {{\n            throw UnableToReadFile::fromLocation($path);\n        }}\n        return $contents;\n    }}\n}}\n"
        )
    };
    let current = class("");
    let legacy = class("/**\n * @deprecated use the V3 adapter\n */\n");
    let pairs = |b: &str| run(&[("v3/A.php", &current, true), ("v2/A.php", b, true)]).pairs;
    assert_eq!(pairs(&current).len(), 1);
    assert!(pairs(&legacy).is_empty());
    // Only a declaration's own header and the lines above it count:
    // another method's decorator, a parameter named `deprecated` or a
    // mark named `deprecated_lifespan` leave the copy a candidate.
    let python = |mark: &str| {
        format!(
            "import json\n\n\nclass Reader:\n    @deprecated(\"use read\")\n    def old(self):\n        return None\n\n{mark}    def load_user(self, path,\n                  deprecated: bool = False):\n        text = open(path).read()\n        value = json.loads(text)\n        name = value[\"name\"].strip().lower().replace(\" \", \"_\")\n        return User(name=name, path=path)\n"
        )
    };
    let current = python("");
    let pairs = |b: &str| run(&[("a.py", &current, true), ("b.py", b, true)]).pairs;
    assert_eq!(pairs(&current).len(), 1);
    assert_eq!(pairs(&python("    @deprecated_lifespan\n")).len(), 1);
    assert!(pairs(&python("    @deprecated(\"use load\")\n")).is_empty());
    assert!(pairs(&python("    @typing_extensions.deprecated(\"x\")\n")).is_empty());
}

#[test]
fn copies_in_retired_directories_are_not_candidates() {
    let pairs = |b: &str| run(&[("src/a.rs", LOAD, true), (b, LOAD, true)]).pairs;
    assert_eq!(pairs("src/b.rs").len(), 1);
    for retired in [
        "Assets/ProofOfConcept/Builder.rs",
        "deprecated/b.rs",
        "scripts/archive/b.rs",
        "src/proof-of-concept/b.rs",
    ] {
        assert!(pairs(retired).is_empty(), "{retired}");
    }
    assert_eq!(
        pairs("src/legacy/b.rs").len(),
        1,
        "legacy code is often live"
    );
}

#[test]
fn renamed_copies_match_across_files_with_statement_aligned_quotes() {
    let renamed = LOAD
        .replace("load_user", "load_team")
        .replace("text", "body")
        .replace("value", "parsed")
        .replace("\"name\"", "\"title\"")
        .replace("User", "Team");
    let found = run(&[("a.rs", LOAD, true), ("b.rs", &renamed, true)]);
    assert_eq!(found.pairs.len(), 1, "{:?}", found.pairs.len());
    let pair = &found.pairs[0];
    assert_eq!(pair.a.path, Path::new("a.rs"));
    assert_eq!(pair.b.path, Path::new("b.rs"));
    assert_eq!(pair.a.function.as_deref(), Some("load_user"));
    assert_eq!(pair.b.function.as_deref(), Some("load_team"));
    assert!(
        pair.a
            .quote
            .starts_with("let text = std::fs::read_to_string")
    );
    assert!(pair.a.quote.ends_with("Ok(User { name })"));
    assert_eq!((pair.a.start_line, pair.a.end_line), (2, 5));
    assert!(pair.differences.contains(&Difference {
        a: "text".into(),
        b: "body".into()
    }));
    assert!(pair.differences.contains(&Difference {
        a: "name".into(),
        b: "title".into()
    }));
    assert_eq!(pair.occurrences, 2);
}

#[test]
fn java_copies_are_candidates_but_equality_boilerplate_is_not() {
    let position = "class Position {\n\tprivate final int line;\n\tprivate final int column;\n\n\t@Override\n\tpublic boolean equals(Object other) {\n\t\tif (this == other) return true;\n\t\tif (other == null || getClass() != other.getClass()) return false;\n\t\tPosition that = (Position) other;\n\t\tif (line != that.line) return false;\n\t\treturn column == that.column;\n\t}\n\n\tString describe(Map<String, String> fields) {\n\t\tString text = fields.get(\"text\");\n\t\tString trimmed = text.trim();\n\t\tString lower = trimmed.toLowerCase();\n\t\tfields.put(\"text\", lower);\n\t\treturn lower + line;\n\t}\n}\n";
    let range = position
        .replace("Position", "Range")
        .replace("line", "start")
        .replace("column", "end");
    let found = run(&[
        ("Position.java", position, true),
        ("Range.java", &range, true),
    ]);
    let functions: Vec<_> = found
        .pairs
        .iter()
        .map(|p| (p.a.function.as_deref(), p.b.function.as_deref()))
        .collect();
    assert_eq!(
        functions,
        [(Some("Position::describe"), Some("Range::describe"))]
    );
}

#[test]
fn java_constructors_filling_their_fields_are_not_copies() {
    let position = "class Position {\n\tPosition(int sourceLineNumber, int sourceColumnNumber, int sourceByteOffset, int sourceCharacterOffset, int trackedPosition) {\n\t\tthis.sourceLineNumber = sourceLineNumber;\n\t\tthis.sourceColumnNumber = sourceColumnNumber;\n\t\tthis.sourceByteOffset = sourceByteOffset;\n\t\tthis.sourceCharacterOffset = sourceCharacterOffset;\n\t\tthis.trackedPosition = trackedPosition;\n\t\tthis.valid = true;\n\t}\n\n\tPosition(Position copy) {\n\t\tsourceLineNumber = copy.sourceLineNumber;\n\t\tsourceColumnNumber = copy.sourceColumnNumber;\n\t\tsourceByteOffset = copy.sourceByteOffset;\n\t\tsourceCharacterOffset = copy.sourceCharacterOffset;\n\t\ttrackedPosition = copy.trackedPosition;\n\t}\n}\n";
    let range = position
        .replace("Position", "Range")
        .replace("line", "start")
        .replace("column", "end");
    assert_eq!(
        pairs_between(("Position.java", position), ("Range.java", &range)),
        0
    );
    // Work beyond storing fields is still compared.
    let worker = |name: &str| {
        format!(
            "class {name} {{\n\t{name}(Map<String, String> fields) {{\n\t\tString text = fields.get(\"text\");\n\t\tString trimmed = text.trim();\n\t\tString lower = trimmed.toLowerCase();\n\t\tfields.put(\"text\", lower);\n\t\tfields.put(\"length\", String.valueOf(lower.length()));\n\t\tthis.fields = fields;\n\t}}\n}}\n"
        )
    };
    let (a, b) = (worker("Position"), worker("Range"));
    assert_eq!(pairs_between(("Position.java", &a), ("Range.java", &b)), 1);
}

#[test]
fn java_setters_given_literals_are_data_not_copies() {
    let fixture = "class OwnerTests {\n\tprivate Owner george() {\n\t\tOwner george = new Owner();\n\t\tgeorge.setFirstName(\"George\");\n\t\tgeorge.setLastName(\"Franklin\");\n\t\tgeorge.setAddress(\"110 W. Liberty St.\");\n\t\tgeorge.setCity(\"Madison\");\n\t\tgeorge.setTelephone(\"6085551023\");\n\t\treturn george;\n\t}\n}\n";
    let inline = "class ServiceTests {\n\tvoid insertsOwner() {\n\t\tOwner owner = new Owner();\n\t\towner.setFirstName(\"Sam\");\n\t\towner.setLastName(\"Schultz\");\n\t\towner.setAddress(\"4, Evans Street\");\n\t\towner.setCity(\"Wollongong\");\n\t\towner.setTelephone(\"4444444444\");\n\t\towners.save(owner);\n\t}\n}\n";
    assert_eq!(
        pairs_between(("OwnerTests.java", fixture), ("ServiceTests.java", inline)),
        0
    );
    // Setters given computed values copy logic and are still compared.
    let mapping = |name: &str| {
        format!(
            "class {name} {{\n\tOwnerDto map(Owner owner) {{\n\t\tOwnerDto dto = new OwnerDto();\n\t\tdto.setFirstName(owner.getFirstName().trim());\n\t\tdto.setLastName(owner.getLastName().trim());\n\t\tdto.setAddress(owner.getAddress().trim());\n\t\tdto.setCity(owner.getCity().toUpperCase());\n\t\tdto.setTelephone(owner.getTelephone().replace(\" \", \"\"));\n\t\treturn dto;\n\t}}\n}}\n"
        )
    };
    let (a, b) = (mapping("OwnerMapper"), mapping("VetMapper"));
    assert_eq!(
        pairs_between(("OwnerMapper.java", &a), ("VetMapper.java", &b)),
        1
    );
}

/// A Ruby assignment's bound locals and a Rust `use` path's imported
/// names: two walks that share only the frame of skipping one kind and
/// recursing into their children.
const BOUND: &str = "fn bound(node: Node<'_>, source: &str, names: &mut Vec<String>) {\n    if node.kind() == \"identifier\" {\n        names.push(text(node, source).to_string());\n        return;\n    }\n    if node.kind() == \"call\" {\n        return;\n    }\n    let mut cursor = node.walk();\n    for child in node.named_children(&mut cursor) {\n        bound(child, source, names);\n    }\n}\n";
const IMPORTS: &str = "fn imports(node: Node<'_>, source: &str, names: &mut BTreeSet<String>) {\n    if node.kind() == \"identifier\" {\n        let name = text(node, source);\n        if !matches!(name, \"self\" | \"super\" | \"crate\") {\n            names.insert(name.to_string());\n        }\n        return;\n    }\n    if node.kind() == \"scoped_identifier\" {\n        if let Some(name) = node.child_by_field_name(\"name\") {\n            imports(name, source, names);\n        }\n        return;\n    }\n    if node.kind() == \"string\" {\n        return;\n    }\n    let mut cursor = node.walk();\n    for child in node.named_children(&mut cursor) {\n        imports(child, source, names);\n    }\n}\n";

#[test]
fn walks_sharing_only_an_early_exit_and_their_recursion_are_not_copies() {
    assert_eq!(
        pairs_between(("ruby.rs", BOUND), ("import_names.rs", IMPORTS)),
        0
    );
    // The same frame around a different exit, after a different first
    // step, with the recursion written as a method on the walker itself,
    // is still only the frame.
    let method = |name: &str, first: &str, stop: &str, call: &str| {
        format!(
            "impl Walker {{\n    fn {name}(&mut self, node: Node<'_>) {{\n        {first}\n        if node.is_missing() {{\n            return;\n        }}\n        if node.kind() == \"{stop}\" {{\n            return;\n        }}\n        let mut cursor = node.walk();\n        for child in node.named_children(&mut cursor) {{\n            if child.is_extra() {{\n                continue;\n            }}\n            self.{call}(child);\n        }}\n    }}\n}}\n"
        )
    };
    let (a, b) = (
        method("locals", "self.depth += 1;", "call", "locals"),
        method(
            "exports",
            "self.seen.insert(node.id());",
            "string",
            "exports",
        ),
    );
    assert_eq!(pairs_between(("a.rs", &a), ("b.rs", &b)), 0);
    // Both calling another method is the same window, and a copy.
    let (a, b) = (
        method("locals", "self.depth += 1;", "call", "visit"),
        method("exports", "self.seen.insert(node.id());", "string", "visit"),
    );
    assert_eq!(pairs_between(("a.rs", &a), ("b.rs", &b)), 1);
    // Exits without braces and `this.` recursion in JavaScript.
    let script = |name: &str, first: &str, stop: &str| {
        format!(
            "class Scanner {{\n  {name}(node) {{\n    {first}\n    if (!node) return;\n    if (node.type === '{stop}') return;\n    const children = node.namedChildren;\n    for (const child of children) {{\n      this.{name}(child);\n    }}\n  }}\n}}\n"
        )
    };
    let (a, b) = (
        script("locals", "this.depth++;", "call_expression"),
        script("exports", "this.seen.add(node.id);", "string"),
    );
    assert_eq!(pairs_between(("a.js", &a), ("b.js", &b)), 0);
}

#[test]
fn walks_copying_the_work_they_do_at_each_node_are_candidates() {
    // Both walks collect the bound identifiers the same way; each calling
    // itself is the same step, so their own names are no difference.
    let copy = BOUND.replace("bound", "assigned");
    assert_eq!(
        differences_between(("ruby.rs", BOUND), ("python.rs", &copy)),
        []
    );
    // A walk that does its work in the loop, around its recursion.
    let visit = |name: &str, first: &str| {
        format!(
            "fn {name}(node: Node<'_>, source: &str, names: &mut Vec<String>) {{\n    {first}\n    if node.kind() == \"call\" {{\n        return;\n    }}\n    let mut cursor = node.walk();\n    for child in node.named_children(&mut cursor) {{\n        if child.kind() == \"identifier\" {{\n            names.push(text(child, source).to_string());\n        }}\n        {name}(child, source, names);\n    }}\n}}\n"
        )
    };
    let (a, b) = (visit("locals", ""), visit("parameters", ""));
    assert_eq!(pairs_between(("a.rs", &a), ("b.rs", &b)), 1);
    // Part of each, after a different first step.
    let (a, b) = (
        visit("locals", "trace(node);"),
        visit("parameters", "names.reserve(8);"),
    );
    assert_eq!(pairs_between(("a.rs", &a), ("b.rs", &b)), 1);
    // Work in a match arm beside the recursion, after a different first
    // step, so that only part of each walk is copied.
    let arms = |name: &str, first: &str| {
        format!(
            "fn {name}(node: Node<'_>, source: &str, names: &mut Vec<String>) {{\n    {first}\n    if node.is_missing() {{\n        return;\n    }}\n    let mut cursor = node.walk();\n    for child in node.named_children(&mut cursor) {{\n        match child.kind() {{\n            \"identifier\" => names.push(child.utf8_text(source.as_bytes()).unwrap().to_string()),\n            \"call\" | \"string\" => {{}}\n            _ => {name}(child, source, names),\n        }}\n    }}\n}}\n"
        )
    };
    let (a, b) = (
        arms("locals", "trace(node);"),
        arms("exports", "names.reserve(8);"),
    );
    assert_eq!(pairs_between(("a.rs", &a), ("b.rs", &b)), 1);
    // Work in a conditional expression beside the recursion.
    let ternary = |name: &str, first: &str| {
        format!(
            "class Walker {{\n  {name}(node, names) {{\n    {first}\n    if (!node) return;\n    if (node.type === 'comment') return;\n    for (const child of node.namedChildren) child.type === 'identifier' ? names.push(child.text.trim().toLowerCase()) : this.{name}(child, names);\n  }}\n}}\n"
        )
    };
    let (a, b) = (
        ternary("collect", "this.depth++;"),
        ternary("gather", "names.clear();"),
    );
    assert_eq!(pairs_between(("a.js", &a), ("b.js", &b)), 1);
}

#[test]
fn whole_copies_of_small_walks_and_guarded_handlers_are_candidates() {
    // One step of work between an exit and the recursion, and no cursor.
    let weights = |name: &str| {
        format!(
            "pub fn {name}(node: &Tree, scale: f64, out: &mut Vec<f64>) {{\n    if node.hidden || node.children.is_empty() {{\n        return;\n    }}\n    out.push(node.weight * scale + node.bias * node.decay.powi(2));\n    for child in &node.children {{\n        {name}(child, scale * node.decay, out);\n    }}\n}}\n"
        )
    };
    let (a, b) = (weights("total_weight"), weights("total_cost"));
    assert_eq!(pairs_between(("weights.rs", &a), ("costs.rs", &b)), 1);
    let labels = |name: &str| {
        format!(
            "def {name}(node, labels):\n    if node is None or node.hidden:\n        return\n    labels.append(node.display_name.strip().lower().replace(\" \", \"_\"))\n    for child in node.children:\n        {name}(child, labels)\n"
        )
    };
    let (a, b) = (labels("collect_names"), labels("gather_labels"));
    assert_eq!(pairs_between(("names.py", &a), ("labels.py", &b)), 1);
    // Exits alone are no walk: handlers that check and notify alike.
    let handler = |name: &str, event: &str| {
        format!(
            "export function {name}({event}) {{\n  if (!{event} || !{event}.repository) return;\n  if ({event}.repository.archived || {event}.repository.disabled) return;\n  if ({event}.sender && {event}.sender.type === 'Bot') return;\n  notifyChannel({event}.repository.fullName, {event}.ref, {event}.headCommit);\n}}\n"
        )
    };
    let (a, b) = (handler("onPush", "event"), handler("onTag", "payload"));
    assert_eq!(pairs_between(("push.js", &a), ("tag.js", &b)), 1);
    // Part of each, with little work after the exits.
    let guarded = |name: &str, first: &str| {
        format!(
            "export function {name}(event) {{\n  {first}\n  if (!event || !event.repository) return;\n  if (event.repository.archived || event.repository.disabled) return;\n  if (event.sender && event.sender.type === 'Bot') return;\n  notify(event);\n}}\n"
        )
    };
    let (a, b) = (
        guarded("onPush", "log.debug('push');"),
        guarded("onTag", "metrics.count('tag', 1);"),
    );
    assert_eq!(pairs_between(("push.js", &a), ("tag.js", &b)), 1);
}

#[test]
fn a_walk_with_one_large_step_of_work_is_a_candidate() {
    let render = |name: &str, first: &str| {
        format!(
            "def {name}(node, out, depth=0):\n    {first}\n    if node is None or node.hidden:\n        return\n    out.write(\"  \" * depth + f\"{{node.kind}} [{{node.start}}..{{node.end}}] {{node.name!r}} ({{len(node.children)}} children)\\n\")\n    for child in node.children:\n        {name}(child, out, depth + 1)\n"
        )
    };
    // The whole of both functions.
    let (a, b) = (render("render", "pass"), render("dump", "pass"));
    assert_eq!(pairs_between(("render.py", &a), ("dump.py", &b)), 1);
    // Part of each, after a different first step.
    let (a, b) = (
        render("render", "depth = depth or 0"),
        render("dump", "out.flush()"),
    );
    assert_eq!(pairs_between(("render.py", &a), ("dump.py", &b)), 1);
}

#[test]
fn bare_calls_in_methods_of_the_same_name_call_another_function() {
    // `dump` inside the method `dump` is the imported `json.dump`.
    let store = |owner: &str, module: &str, name: &str| {
        format!(
            "from {module} import {name}\n\n\nclass {owner}:\n    def {name}(self, record):\n        if record is None:\n            return\n        payload = {{\"id\": record.id, \"name\": record.name.strip(), \"tags\": sorted(record.tags), \"owner\": record.owner.email}}\n        {name}(payload, self.handle, indent=2, sort_keys=True)\n"
        )
    };
    let (a, b) = (
        store("JsonStore", "json", "dump"),
        store("YamlStore", "yaml", "safe_dump"),
    );
    let renamed = Difference {
        a: "dump".into(),
        b: "safe_dump".into(),
    };
    assert!(differences_between(("json_store.py", &a), ("yaml_store.py", &b)).contains(&renamed));
    // A Rust method calling a function of another module by its own name.
    let cache = |owner: &str, name: &str| {
        format!(
            "impl {owner} {{\n    fn {name}(&self, key: &str) -> Option<String> {{\n        let path = self.root.join(key.trim_start_matches('/'));\n        let value = super::store::{name}(&path)?;\n        self.hits.fetch_add(1, Ordering::Relaxed);\n        Some(value.trim().to_string())\n    }}\n}}\n"
        )
    };
    let (a, b) = (cache("Cache", "get"), cache("Mirror", "fetch"));
    let renamed = Difference {
        a: "get".into(),
        b: "fetch".into(),
    };
    assert!(differences_between(("cache.rs", &a), ("mirror.rs", &b)).contains(&renamed));
}

#[test]
fn go_functions_sharing_only_error_checks_and_cleanups_are_not_copies() {
    let store = |name: &str, find: &str, kind: &str, tail: &str| {
        format!(
            "package sqlite\n\nfunc (s *Service) {name}(ctx context.Context, id int) (*wtf.{kind}, error) {{\n\ttx, err := s.db.BeginTransactionWithOptions(ctx, nil)\n\tif err != nil {{\n\t\treturn nil, err\n\t}}\n\tdefer tx.Rollback()\n\trecord, err := {find}(ctx, tx, id)\n\tif err != nil {{\n\t\treturn nil, err\n\t}}\n\t{tail}\n\treturn record, nil\n}}\n"
        )
    };
    let (a, b) = (
        store(
            "FindAuthByID",
            "findAuthByID",
            "Auth",
            "record.LastSeen = time.Now()",
        ),
        store(
            "FindDialByID",
            "findDialByID",
            "Dial",
            "go notify(record.ID, s.events)",
        ),
    );
    assert_eq!(pairs_between(("auth.go", &a), ("dial.go", &b)), 0);
    // With more work between them, the error checks do not hide a copy.
    let (a, b) = (
        a.replace("\trecord, err", "\tlog.Printf(\"loading one stored record by its identifier\")\n\tmetrics.Count(\"store.find\", 1)\n\trecord, err"),
        b.replace("\trecord, err", "\tlog.Printf(\"loading one stored record by its identifier\")\n\tmetrics.Count(\"store.find\", 1)\n\trecord, err"),
    );
    assert_eq!(pairs_between(("auth.go", &a), ("dial.go", &b)), 1);
}

#[test]
fn a_list_of_alike_statements_is_not_a_copy_of_itself() {
    let source = "def create(self, a=None, b=None, c=None, d=None, e=None, f=None):\n    a = self.value_or_default(\"alpha_setting\", a)\n    b = self.value_or_default(\"beta_setting\", b)\n    c = self.value_or_default(\"gamma_setting\", c)\n    d = self.value_or_default(\"delta_setting\", d)\n    e = self.value_or_default(\"epsilon_setting\", e)\n    f = self.value_or_default(\"zeta_setting\", f)\n    return self.build(a, b, c, d, e, f)\n";
    assert!(run(&[("db.py", source, true)]).pairs.is_empty());
}

#[test]
fn variants_of_one_example_are_not_compared() {
    let flow = |dir: &str| format!("examples/login/{dir}/apis.py");
    assert!(super::separate_examples(
        Path::new(&flow("raw")),
        Path::new(&flow("sdk"))
    ));
    assert!(super::separate_examples(
        Path::new("app/blog_examples/raw/views.py"),
        Path::new("app/blog_examples/sdk/views.py")
    ));
    assert!(!super::separate_examples(
        Path::new("examples/login/raw/apis.py"),
        Path::new("examples/login/raw/views.py")
    ));
    assert!(!super::separate_examples(
        Path::new("src/billing/raw/apis.py"),
        Path::new("src/billing/sdk/apis.py")
    ));
    // Benchmark programs side by side; one benchmark suite's files are one program.
    assert!(super::separate_examples(
        Path::new("bench/runtime/nbody/main.bend"),
        Path::new("bench/runtime/mandelbrot/main.bend")
    ));
    assert!(!super::separate_examples(
        Path::new("benchmarks/multipart_benchmark.py"),
        Path::new("benchmarks/urlencoded_benchmark.py")
    ));
    for path in [
        "docs_src/tutorial/one/tutorial001.py",
        "example/settings.py",
        "examples/hello_world.rs",
    ] {
        assert!(super::example_code(Path::new(path)), "{path}");
    }
    assert!(!super::example_code(Path::new("src/examples.rs")));
    for path in [
        "samples/MediatR.Examples/Runner.cs",
        "src/MediatR.Examples.Autofac/Program.cs",
        "example_authentication_middleware_test.go",
    ] {
        assert!(super::example_code(Path::new(path)), "{path}");
    }
    assert!(!super::example_code(Path::new(
        "src/main/java/org/springframework/samples/petclinic/Owner.java"
    )));
    for path in [
        "src/main/java/com/example/demo/OrderController.java",
        "service/src/test/kotlin/com/example/OrderTest.kt",
    ] {
        assert!(!super::example_code(Path::new(path)), "{path}");
    }
    assert!(super::example_code(Path::new(
        "examples/spring/src/main/java/com/example/demo/Main.java"
    )));
}

#[test]
fn inconsistent_renaming_and_short_windows_are_rejected() {
    // `text` becomes two different names on the other side.
    let inconsistent = LOAD
        .replace("let text", "let body")
        .replace("from_str(&text)", "from_str(&other)");
    assert!(
        run(&[("a.rs", LOAD, true), ("b.rs", &inconsistent, true)])
            .pairs
            .is_empty()
    );
    let short = "fn a(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\nfn b(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\n";
    assert!(run(&[("s.rs", short, true)]).pairs.is_empty());
    // A docstring is not a statement: two statements stay too few.
    let wrappers = "def render_template(name, **context):\n    \"\"\"Render a template by name with the given context and return the resulting page.\"\"\"\n    app = current_app._get_current_object()\n    return _render(app, app.jinja_env.get_or_select_template(name), context)\n\n\ndef stream_template(name, **context):\n    \"\"\"Render a template by name with the given context as a stream of page parts.\"\"\"\n    app = current_app._get_current_object()\n    return _stream(app, app.jinja_env.get_or_select_template(name), context)\n";
    assert!(run(&[("templating.py", wrappers, true)]).pairs.is_empty());
}

#[test]
fn a_short_idiom_inside_a_larger_copy_forms_its_own_group() {
    let head = "    let text = std::fs::read_to_string(path).expect(\"reading the configured user file failed\");\n    let value: Value = serde_json::from_str(&text).expect(\"parsing the configured user file failed\");\n    let root = value.as_object().expect(\"the configured user file holds an object\");\n";
    let tail = "    let name = root[\"name\"].as_str().unwrap_or(\"anonymous\").trim().to_string();\n    let age = root[\"age\"].as_u64().unwrap_or(0).min(150) as u32;\n    let city = root[\"city\"].as_str().unwrap_or(\"unknown\").trim().to_string();\n    let email = root[\"email\"].as_str().unwrap_or(\"\").trim().to_lowercase();\n    Ok(User { name, age, city, email })\n";
    let full = format!("fn load(path: &str) -> Result<User> {{\n{head}{tail}}}\n");
    let other = full.replace("fn load", "fn again");
    let short = format!("fn count(path: &str) -> usize {{\n{head}    root.len()\n}}\n");
    let found = run(&[
        ("a.rs", &full, true),
        ("b.rs", &other, true),
        ("c.rs", &short, true),
    ]);
    let largest = found.pairs.iter().max_by_key(|p| p.size).unwrap();
    assert_eq!(
        (largest.a.path.as_path(), largest.b.path.as_path()),
        (Path::new("a.rs"), Path::new("b.rs"))
    );
    assert!(largest.copies.is_empty(), "{:?}", largest.copies);
    assert_eq!(found.pairs.len(), 2);
}

#[test]
fn context_only_pairs_are_excluded_but_selected_to_context_pairs_are_kept() {
    let copy = LOAD.replace("load_user", "load_again");
    assert!(
        run(&[("a.rs", LOAD, false), ("b.rs", &copy, false)])
            .pairs
            .is_empty()
    );
    let found = run(&[("context.rs", LOAD, false), ("selected.rs", &copy, true)]);
    assert_eq!(found.pairs.len(), 1);
    assert_eq!(found.pairs[0].a.path, Path::new("selected.rs"));
    assert_eq!(found.pairs[0].b.path, Path::new("context.rs"));
}

#[test]
fn repeated_copies_form_one_group_judged_through_one_pair() {
    let three = [
        LOAD,
        &LOAD.replace("load_user", "second"),
        &LOAD.replace("load_user", "third"),
    ]
    .concat();
    let found = run(&[("three.rs", &three, true)]);
    assert_eq!(found.pairs.len(), 1);
    let pair = &found.pairs[0];
    assert_eq!((pair.occurrences, pair.a.start_line), (3, 2));
    assert_eq!(pair.copies.len(), 1);
    let lines = [pair.b.start_line, pair.copies[0].start_line];
    assert!(lines.contains(&8) && lines.contains(&14), "{lines:?}");

    // Copies of different lengths still share a group through overlapping sites.
    let longer = LOAD.replace(
        "    Ok(User { name })",
        "    let checked = name.trim().to_string();\n    Ok(User { name: checked })",
    );
    let found = run(&[
        ("a.rs", LOAD, true),
        ("b.rs", &LOAD.replace("load_user", "other"), true),
        ("c.rs", &longer.replace("load_user", "third"), true),
    ]);
    assert_eq!(
        found.pairs.len(),
        1,
        "{:?}",
        found
            .pairs
            .iter()
            .map(|p| (&p.a.path, &p.b.path))
            .collect::<Vec<_>>()
    );
    assert_eq!(found.pairs[0].occurrences, 3);
    let body = "    total = decimal.Decimal(\"0\")\n    for row in rows:\n        total += row.amount * row.exchange_rate - row.discount_amount\n    return total.quantize(decimal.Decimal(\"0.01\"), rounding=decimal.ROUND_HALF_UP)\n";
    let python = format!(
        "def a(rows):\n{body}\ndef b(items):\n{}",
        body.replace("row", "item")
    );
    let found = run(&[("totals.py", &python, true)]);
    assert_eq!(found.pairs.len(), 1);
    assert_eq!(found.pairs[0].a.function.as_deref(), Some("a"));
}

#[test]
fn windows_of_the_same_two_functions_split_by_one_statement_are_one_pair() {
    let head = "    parser = argparse.ArgumentParser(description=__doc__)\n    parser.add_argument('--owner', default='Tech')\n    parser.add_argument('--project', type=int, default=2)\n    parser.add_argument('--apply', action='store_true')\n";
    let tail = "    args = parser.parse_args()\n    if args.apply and not args.backup:\n        parser.error('--apply requires --backup')\n    run(args.owner, args.project, args.apply, args.backup)\n";
    let first = format!(
        "def main():\n{head}    parser.add_argument('--completed', action='store_true')\n{tail}"
    );
    let second = format!("def main():\n{head}{tail}");
    let found = run(&[("migrate.py", &first, true), ("retire.py", &second, true)]);
    assert_eq!(
        found
            .pairs
            .iter()
            .map(|p| (p.a.start_line, p.b.start_line))
            .collect::<Vec<_>>()
            .len(),
        1
    );
}

#[test]
fn copies_pair_within_a_generic_language_s_family_only() {
    let kotlin = |name: &str, value: &str| {
        format!(
            "fun {name}(items: List<Item>, discount: Int): Int {{\n    val open = items.filter {{ it.open && it.price > discount }}\n    val total = open.sumOf {{ it.price * {value} - discount }}\n    logger.info(\"total $total for ${{open.size}} open items\")\n    return total + open.size * discount\n}}\n"
        )
    };
    let (a, b) = (kotlin("openTotal", "2"), kotlin("closedTotal", "3"));
    let differences = differences_between(("a/Open.kt", &a), ("a/Closed.kt", &b));
    assert_eq!(
        differences,
        [Difference {
            a: "3".into(),
            b: "2".into()
        }]
    );
    // The same statements in C, C++ and Java: C and C++ are one family.
    let body = "    int total = 0;\n    for (int i = 0; i < count; i++) {\n        total += prices[i] * weights[i] - discounts[i];\n    }\n    printf(\"%d items weigh %d in all\", count, total);\n    return total + count * shipping;\n";
    let c = format!("int sum(int *prices, int *weights, int count) {{\n{body}}}\n");
    let java = format!(
        "class Sum {{\n  int sum(int[] prices, int[] weights, int count) {{\n{body}  }}\n}}\n"
    );
    assert_eq!(pairs_between(("a/sum.c", &c), ("a/sum.cpp", &c)), 1);
    assert_eq!(pairs_between(("a/sum.c", &c), ("a/Sum.java", &java)), 0);
}
