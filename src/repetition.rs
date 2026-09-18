//! Repeated syntax tokens locate evidence; they do not establish shared responsibility.
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
use tree_sitter::Node;

struct Token<'a> {
    text: &'a str,
    start: usize,
    end: usize,
    line: usize,
    end_line: usize,
}

fn tokens<'a>(node: Node<'_>, source: &'a str, output: &mut Vec<Token<'a>>) {
    if node.kind().contains("comment") {
        return;
    }
    if node.child_count() == 0 || node.kind().contains("string") {
        output.push(Token {
            text: &source[node.byte_range()],
            start: node.start_byte(),
            end: node.end_byte(),
            line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        tokens(child, source, output);
    }
}

pub fn observations<'a>(sources: impl Iterator<Item = (&'a Path, &'a str)>) -> Result<Value> {
    let sources: Vec<_> = sources.collect();
    let mut streams = Vec::new();
    for (path, source) in &sources {
        let mut stream = Vec::new();
        if let Some(tree) = crate::locations::parse(path, source)? {
            tokens(tree.root_node(), source, &mut stream);
        }
        streams.push(stream);
    }
    let mut groups = BTreeMap::<Vec<&str>, Vec<(usize, usize)>>::new();
    for (file, stream) in streams.iter().enumerate() {
        for (start, window) in stream.windows(12).enumerate() {
            groups
                .entry(window.iter().map(|t| t.text).collect())
                .or_default()
                .push((file, start));
        }
    }
    let mut candidates = Vec::new();
    for occurrences in groups
        .values()
        .filter(|v| v.len() > 1 && v.iter().any(|(file, _)| *file == 0))
    {
        let (file, start) = occurrences[0];
        let mut length = 12;
        while length < 200 {
            let Some(next) = streams[file].get(start + length) else {
                break;
            };
            if next.end - streams[file][start].start > 1200
                || !occurrences.iter().all(|(f, s)| {
                    streams[*f]
                        .get(s + length)
                        .is_some_and(|t| t.text == next.text)
                })
            {
                break;
            }
            length += 1;
        }
        let first = &streams[file][start];
        let last = &streams[file][start + length - 1];
        if !(60..=1200).contains(&(last.end - first.start)) {
            continue;
        }
        let locations: Vec<_> = occurrences.iter().take(20).map(|(f,s)| {
            let first = &streams[*f][*s]; let last = &streams[*f][s+length-1];
            json!({"path":sources[*f].0,"start_byte":first.start,"end_byte":last.end,"start_line":first.line,"end_line":last.end_line})
        }).collect();
        let mut candidate =
            json!({"source":&sources[file].1[first.start..last.end],"locations":locations});
        if occurrences.len() > 20 {
            candidate["locations_omitted"] = json!(occurrences.len() - 20);
        }
        candidates.push((length, candidate));
    }
    candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
    let total = candidates.len();
    let mut selected = Vec::<Value>::new();
    let mut covered = Vec::<Value>::new();
    for (_, candidate) in candidates {
        let locations = candidate["locations"].as_array().unwrap();
        if locations.iter().all(|location| {
            covered.iter().any(|outer| {
                outer["path"] == location["path"]
                    && outer["start_byte"].as_u64() <= location["start_byte"].as_u64()
                    && outer["end_byte"].as_u64() >= location["end_byte"].as_u64()
            })
        }) {
            continue;
        }
        covered.extend(locations.iter().cloned());
        selected.push(candidate);
        if selected.len() == 12 {
            break;
        }
    }
    Ok(
        json!({"observations":selected,"candidate_count":total,"scope":"Up to 12 longest repeated token spans involving the primary file; whitespace ignored, literal contents preserved. These may be partial expressions, declarations or harmless idioms. Repetition is not a semantic verdict; absence does not establish no duplication."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whitespace_variants_match_without_erasing_literal_values_or_source_locations() {
        let a = "fn a(){ client.timeout(100).retries(3).proxy(\"https://example.test\"); }";
        let b =
            "fn b() {\n client . timeout(100) . retries(3) . proxy(\"https://example.test\");\n}";
        let result =
            observations([(Path::new("a.rs"), a), (Path::new("b.rs"), b)].into_iter()).unwrap();
        assert!(!result["observations"].as_array().unwrap().is_empty());
        for candidate in result["observations"].as_array().unwrap() {
            assert!(
                candidate["locations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|l| l["path"] == "a.rs")
            );
            for location in candidate["locations"].as_array().unwrap() {
                let source = if location["path"] == "a.rs" { a } else { b };
                let range = location["start_byte"].as_u64().unwrap() as usize
                    ..location["end_byte"].as_u64().unwrap() as usize;
                assert!(source.get(range).is_some());
            }
        }
        let changed = b
            .replace("100", "200")
            .replace("retries(3)", "retries(4)")
            .replace("example.test", "different.test");
        assert!(
            observations(
                [
                    (Path::new("a.rs"), a),
                    (Path::new("b.rs"), changed.as_str())
                ]
                .into_iter()
            )
            .unwrap()["observations"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn context_only_matches_are_not_candidates() {
        let primary = "fn primary() {}";
        let peer = "fn a(){ client.timeout(100).retries(3).proxy(\"https://example.test\"); }";
        let result = observations(
            [
                (Path::new("main.rs"), primary),
                (Path::new("a.rs"), peer),
                (Path::new("b.rs"), peer),
            ]
            .into_iter(),
        )
        .unwrap();
        assert!(result["observations"].as_array().unwrap().is_empty());
    }
    #[test]
    fn repeated_input_bounds_locations_and_discloses_omissions() {
        let source = format!(
            "fn repeated() {{ {} }}",
            "client.timeout(100).retries(3).proxy(\"https://example.test\");\n".repeat(80)
        );
        let result = observations([(Path::new("a.rs"), source.as_str())].into_iter()).unwrap();
        let candidates = result["observations"].as_array().unwrap();
        assert!(!candidates.is_empty());
        assert!(candidates.len() <= 12);
        assert!(
            candidates
                .iter()
                .all(|c| c["locations"].as_array().unwrap().len() <= 20)
        );
        assert!(
            candidates
                .iter()
                .any(|c| c["locations_omitted"].as_u64().unwrap_or(0) > 0)
        );
    }
}
