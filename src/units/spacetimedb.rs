//! Access control in SpacetimeDB TypeScript modules: the files that import
//! `spacetimedb/server`. Every client reads public tables, subscribes to
//! views and calls reducers with arguments of its choice, so a public table
//! of one player's data, a view that returns other players' rows, or a
//! reducer that changes rows its arguments choose without checking the
//! caller exposes other players. Each definition is asked about with the
//! functions it calls and the framework facts of the module's version.
use super::{Access, Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan};
use super::{compact, identity, questions};
use crate::{catalog::ACCESS_CONTROL, schema::Pass};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Whether a source file is a SpacetimeDB module, by its server import.
pub fn spacetimedb_module(source: &str) -> bool {
    source.contains("'spacetimedb/server'") || source.contains("\"spacetimedb/server\"")
}

/// Helpers shown per definition, found by the calls in its source and theirs.
const HELPERS: usize = 6;
const HELPER_DEPTH: usize = 2;
/// A longer helper is left out rather than cut.
const HELPER_BYTES: usize = 4000;

/// A table, view or reducer as written: `const NAME = table(…)`,
/// `const NAME = db.view(…)` or `const NAME = db.reducer(…)`.
struct Definition<'a> {
    access: Access,
    name: &'a str,
    source: &'a str,
    start_line: usize,
    end_line: usize,
}

/// Definitions in source order. Private tables are left out: clients cannot
/// read them, so there is nothing to ask.
fn definitions(source: &str) -> Vec<Definition<'_>> {
    let mut found = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let declaration = line.trim_start().trim_start_matches("export ");
        let Some(rest) = declaration.strip_prefix("const ") else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        let name = name.split(':').next().unwrap_or("").trim();
        let value = value.trim_start();
        let callee = value.split('(').next().unwrap_or("");
        let access = match callee.rsplit('.').next() {
            Some("table") if callee == "table" => Access::Table,
            Some("view") if callee.contains('.') => Access::View,
            Some("reducer") if callee.contains('.') => Access::Reducer,
            _ => continue,
        };
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        {
            continue;
        }
        let open = start + (value.as_ptr() as usize - line.as_ptr() as usize) + callee.len();
        let Some(end) = closing(source, open) else {
            continue;
        };
        let text = &source[start..end];
        let text = text.trim_start();
        if access == Access::Table && !first_argument(text).contains("public: true") {
            continue;
        }
        found.push(Definition {
            access,
            name,
            source: text,
            start_line: crate::analysis::line_of(source, start),
            end_line: crate::analysis::line_of(source, end.saturating_sub(1)),
        });
    }
    found
}

/// The byte after the parenthesis that closes the one at `open`.
fn closing(source: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in source[open..].char_indices() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// The text of a call's first argument, such as a table's options.
fn first_argument(call: &str) -> &str {
    let Some(open) = call.find('(') else {
        return "";
    };
    let rest = &call[open + 1..];
    let mut depth = 0usize;
    for (i, c) in rest.char_indices() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => return &rest[..i],
            _ => {}
        }
    }
    rest
}

/// Columns that hold an identity, or a character, owner, account, player or
/// user id: evidence of whose rows a table holds.
fn player_columns(table: &str) -> Vec<String> {
    let mut columns = Vec::new();
    for (at, _) in table.match_indices(": t.") {
        let name: String = table[..at]
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let kind: String = table[at + 4..]
            .chars()
            .take_while(|c| c.is_alphanumeric())
            .collect();
        let lower = name.to_lowercase();
        let owner = ["character", "owner", "account", "player", "user"]
            .iter()
            .any(|word| lower == *word || lower == format!("{word}id"));
        if !name.is_empty() && (kind == "identity" || owner) && !columns.contains(&name) {
            columns.push(name);
        }
    }
    columns
}

/// A function of the module, for the helpers a definition calls.
pub(super) struct Helper {
    pub name: String,
    pub path: PathBuf,
    pub source_hash: String,
    pub source: String,
}

/// Functions a definition calls, and those they call, found by name.
fn helpers<'a>(source: &str, lookup: &impl Fn(&str) -> Option<&'a Helper>) -> Vec<&'a Helper> {
    let mut found: Vec<&Helper> = Vec::new();
    let mut frontier = vec![source.to_string()];
    for _ in 0..HELPER_DEPTH {
        let mut next = Vec::new();
        for text in &frontier {
            for name in called_names(text) {
                if found.len() == HELPERS || found.iter().any(|h| h.name == name) {
                    continue;
                }
                if let Some(helper) = lookup(name).filter(|h| h.source.len() <= HELPER_BYTES) {
                    found.push(helper);
                    next.push(helper.source.clone());
                }
            }
        }
        frontier = next;
    }
    found
}

/// Names followed by `(` in `text`.
fn called_names(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    for (at, _) in text.match_indices('(') {
        let before = &text[..at];
        let start = before
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .map_or(0, |i| i + 1);
        let name = &before[start..];
        if !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

/// Plan one module file's tables, views and reducers. `lookup` finds a
/// function of the same package by name; `version` is the `spacetimedb`
/// version its manifest declares, empty when unknown.
pub(super) fn plan<'a>(
    file: &FileContext<'_>,
    version: &str,
    lookup: impl Fn(&str) -> Option<&'a Helper>,
    out: &mut FilePlan,
    requests: &mut Vec<Planned>,
) {
    let framework = questions::spacetimedb_framework(version);
    out.rules.insert(ACCESS_CONTROL, 0);
    for definition in definitions(file.source) {
        let (key, asked): (&str, Vec<(&'static str, Value)>) = match definition.access {
            Access::Table => (
                "table",
                vec![
                    ("data", questions::stdb_table_data(&framework)),
                    ("exposed", questions::stdb_table_exposed(&framework)),
                ],
            ),
            Access::View => (
                "view",
                vec![
                    ("rows", questions::stdb_view_rows(&framework)),
                    ("returns_others", questions::stdb_view_others(&framework)),
                ],
            ),
            _ => (
                "reducer",
                vec![
                    ("reach", questions::stdb_reducer_reach(&framework)),
                    (
                        "argument_rows",
                        questions::stdb_reducer_argument_rows(&framework),
                    ),
                    (
                        "operator_only",
                        questions::stdb_reducer_operator_only(&framework),
                    ),
                ],
            ),
        };
        let id = format!("{key}:{}", definition.name);
        let mut questions = Questions::default();
        for (question, body) in asked {
            questions.ask(
                question.into(),
                body,
                &id,
                ACCESS_CONTROL,
                question,
                Pass::First,
            );
        }
        let mut state = json!({"file": file.file_state()});
        let mut sources: Vec<(&Path, &str)> = vec![(file.path, file.source_hash)];
        if definition.access == Access::Table {
            state["table"] = json!({
                "name": definition.name,
                "source": definition.source,
                "columns_naming_players": player_columns(definition.source),
            });
        } else {
            state[key] = json!({"name": definition.name, "source": definition.source});
            let found = helpers(definition.source, &lookup);
            state["helpers"] = json!(
                found
                    .iter()
                    .map(|h| json!({"name": h.name, "source": h.source}))
                    .collect::<Vec<_>>()
            );
            for helper in &found {
                if !sources.iter().any(|(p, _)| *p == helper.path) {
                    sources.push((&helper.path, &helper.source_hash));
                }
            }
        }
        let (request, asked) = super::request(file.model, "access", &sources, state, questions);
        let fits = file.budget.fits(&request);
        out.units.push(UnitPlan {
            rule: ACCESS_CONTROL,
            id: id.clone(),
            name: definition.name.to_string(),
            presence: if fits {
                Presence::Judged
            } else {
                Presence::NeedsContext
            },
            locations: vec![file.location(
                definition.start_line,
                definition.end_line,
                Some(definition.name),
            )],
            quote: None,
            lines: definition.end_line + 1 - definition.start_line,
            identity: identity(&[&id, &compact(definition.source)]),
            detail: Detail::Access(definition.access),
            recheck: None,
        });
        if fits {
            requests.push(Planned {
                owner: file.owner,
                request,
                asked,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODULE: &str = "import { schema, table, t } from 'spacetimedb/server'\n\nexport const character = table(\n  { public: true },\n  {\n    id: t.u64().primaryKey(),\n    owner: t.identity(),\n    accountId: t.u64(),\n    name: t.string(),\n  },\n)\n\nexport const account = table({ public: false }, { owner: t.identity().primaryKey() })\n\nexport const myCharacters = database.view({ public: true }, t.array(character.rowType), (ctx) => {\n  return ownedCharacters(ctx)\n})\n\nexport const renameCharacter = database.reducer({ characterId: t.u64(), name: t.string() }, (ctx, { characterId, name }) => {\n  const row = ctx.db.character.id.find(characterId)!\n  ctx.db.character.id.update({ ...row, name })\n})\n";

    #[test]
    fn public_tables_views_and_reducers_are_found_with_their_source() {
        let found = definitions(MODULE);
        let kinds: Vec<(&Access, &str, usize, usize)> = found
            .iter()
            .map(|d| (&d.access, d.name, d.start_line, d.end_line))
            .collect();
        assert_eq!(
            kinds,
            [
                (&Access::Table, "character", 3, 11),
                (&Access::View, "myCharacters", 15, 17),
                (&Access::Reducer, "renameCharacter", 19, 22),
            ]
        );
        assert!(found[2].source.ends_with("update({ ...row, name })\n})"));
        assert_eq!(player_columns(found[0].source), ["owner", "accountId"]);
        assert!(spacetimedb_module(MODULE));
        assert!(!spacetimedb_module(
            "import { t } from './spacetimedb/server-types'"
        ));
    }

    #[test]
    fn helpers_follow_calls_two_deep_by_name() {
        let owned = Helper {
            name: "ownedCharacters".into(),
            path: "src/characters/owned.ts".into(),
            source_hash: "h".into(),
            source: "function ownedCharacters(ctx) {\n  const owner = requirePlayer(ctx)\n  return [...ctx.db.character.owner.filter(owner)]\n}".into(),
        };
        let player = Helper {
            name: "requirePlayer".into(),
            path: "src/access/authorize.ts".into(),
            source_hash: "h".into(),
            source: "export function requirePlayer(ctx) {\n  return ctx.sender\n}".into(),
        };
        let lookup = |name: &str| match name {
            "ownedCharacters" => Some(&owned),
            "requirePlayer" => Some(&player),
            _ => None,
        };
        let found = helpers("(ctx) => { return ownedCharacters(ctx) }", &lookup);
        let names: Vec<&str> = found.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["ownedCharacters", "requirePlayer"]);
    }
}
