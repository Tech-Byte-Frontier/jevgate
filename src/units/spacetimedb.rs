//! Access control in SpacetimeDB modules, whatever the application: TypeScript
//! files that import `spacetimedb/server` and Rust files that use
//! `spacetimedb` attributes. Every client reads public tables, subscribes to
//! views and calls reducers with arguments of its choice, so a public table
//! of one user's data, a view that returns other users' rows, or a
//! reducer that changes rows its arguments choose without checking the
//! caller exposes other users. Each definition is asked about with the
//! functions it calls and the framework facts of the module's version.
use super::{Access, Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan};
use super::{compact, identity, questions};
use crate::{catalog::ACCESS_CONTROL, schema::Pass};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Whether a source file is a SpacetimeDB module: a TypeScript server
/// import, or Rust that uses `spacetimedb` and declares a table or reducer.
pub fn spacetimedb_module(source: &str) -> bool {
    source.contains("'spacetimedb/server'")
        || source.contains("\"spacetimedb/server\"")
        || (source.contains("use spacetimedb")
            && RUST_ATTRIBUTES
                .iter()
                .any(|attribute| source.contains(attribute)))
}

/// Attributes that declare Rust module definitions.
const RUST_ATTRIBUTES: [&str; 6] = [
    "#[spacetimedb::table(",
    "#[table(",
    "#[spacetimedb::reducer",
    "#[reducer",
    "#[spacetimedb::view(",
    "#[view(",
];

/// Lifecycle reducers that only the database calls.
const LIFECYCLE: [&str; 3] = ["init", "client_connected", "client_disconnected"];

/// Column names, with or without an `id` suffix, that tie a row to a user.
const OWNER_WORDS: [&str; 7] = [
    "user",
    "owner",
    "account",
    "member",
    "customer",
    "player",
    "character",
];

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
    source: std::borrow::Cow<'a, str>,
    start_line: usize,
    end_line: usize,
}

/// Definitions in source order, of a TypeScript or Rust module.
fn definitions<'a>(path: &Path, source: &'a str) -> Vec<Definition<'a>> {
    if path.extension().is_some_and(|e| e == "rs") {
        rust_definitions(source)
    } else {
        typescript_definitions(source)
    }
}

/// Rust definitions: each public `#[table(…)]` of a struct (a struct can
/// declare a public and a private table), each `#[reducer]` other than the
/// lifecycle ones, and each `#[view(…)]`, with their attributes. A table's
/// source keeps only its own table attribute.
fn rust_definitions(source: &str) -> Vec<Definition<'_>> {
    let mut found = Vec::new();
    for item in rust_items(source) {
        found.extend(public_tables(&item));
        found.extend(callable(&item, source));
    }
    found
}

/// A Rust item with the attributes above it.
struct RustItem<'a> {
    attributes: Vec<&'a str>,
    /// Byte offset of the first attribute.
    first: usize,
    /// The item from its first line through its closing brace or `;`.
    body: &'a str,
    end: usize,
    start_line: usize,
    end_line: usize,
}

/// Items that carry attributes, in source order.
fn rust_items(source: &str) -> Vec<RustItem<'_>> {
    let mut items = Vec::new();
    let mut attributes: Vec<(usize, &str)> = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let trimmed = line.trim();
        if trimmed.starts_with("#[") {
            attributes.push((start, trimmed));
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let taken = std::mem::take(&mut attributes);
        let Some(&(first, _)) = taken.first() else {
            continue;
        };
        let end = rust_item_end(source, start).unwrap_or(offset);
        items.push(RustItem {
            attributes: taken.into_iter().map(|(_, a)| a).collect(),
            first,
            body: source[start..end].trim_end(),
            end,
            start_line: crate::analysis::line_of(source, first),
            end_line: crate::analysis::line_of(source, end.saturating_sub(1)),
        });
    }
    items
}

/// One definition per public `#[table(…)]` of the item, its source keeping
/// only that table attribute beside the item's other attributes.
fn public_tables<'a>(item: &RustItem<'a>) -> Vec<Definition<'a>> {
    let others: Vec<&str> = item
        .attributes
        .iter()
        .copied()
        .filter(|a| !attribute_named(a, "table"))
        .collect();
    item.attributes
        .iter()
        .filter(|a| attribute_named(a, "table"))
        .filter_map(|attribute| {
            let arguments = attribute_arguments(attribute);
            if !arguments.contains(&"public") {
                return None;
            }
            let name = arguments.iter().find_map(|a| {
                let (key, value) = a.split_once('=')?;
                matches!(key.trim(), "accessor" | "name").then(|| value.trim().trim_matches('"'))
            })?;
            let mut text = String::from(*attribute);
            for other in &others {
                text.push('\n');
                text.push_str(other);
            }
            text.push('\n');
            text.push_str(item.body);
            Some(Definition {
                access: Access::Table,
                name,
                source: text.into(),
                start_line: item.start_line,
                end_line: item.end_line,
            })
        })
        .collect()
}

/// A view, or a reducer other than a lifecycle one, with its attributes.
fn callable<'a>(item: &RustItem<'a>, source: &'a str) -> Option<Definition<'a>> {
    let named = |kind: &str| item.attributes.iter().any(|a| attribute_named(a, kind));
    let lifecycle = item.attributes.iter().any(|a| {
        attribute_named(a, "reducer")
            && attribute_arguments(a).iter().any(|x| LIFECYCLE.contains(x))
    });
    let access = if named("view") {
        Access::View
    } else if named("reducer") && !lifecycle {
        Access::Reducer
    } else {
        return None;
    };
    let name = item
        .body
        .split_once("fn ")
        .map(|(_, rest)| rest.split(['(', '<']).next().unwrap_or("").trim())
        .filter(|n| !n.is_empty())?;
    Some(Definition {
        access,
        name,
        source: source[item.first..item.end].trim_end().into(),
        start_line: item.start_line,
        end_line: item.end_line,
    })
}

/// The table line that schedules this reducer: a Rust `#[table(…,
/// scheduled(name))]` attribute or a TypeScript `scheduled: 'name'` or
/// `scheduled: () => name` option. The schedule is declared on the table, not
/// on the reducer, and in 1.x a client can call the reducer too.
fn scheduled_by<'a>(source: &'a str, reducer: &str) -> Option<&'a str> {
    source.lines().map(str::trim).find(|line| {
        typescript_schedule(line, reducer)
            || attribute_named(line, "table")
                && attribute_arguments(line).iter().any(|a| {
                    a.strip_prefix("scheduled(")
                        .and_then(|rest| rest.strip_suffix(')'))
                        .is_some_and(|name| name.trim() == reducer)
                })
    })
}

/// Whether a TypeScript table option line schedules `reducer`.
fn typescript_schedule(line: &str, reducer: &str) -> bool {
    line.split_once("scheduled:").is_some_and(|(_, value)| {
        let value = value.trim_start().trim_start_matches("() =>").trim_start();
        let value = value.trim_start_matches(['\'', '"']);
        value
            .strip_prefix(reducer)
            .is_some_and(|rest| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
    })
}

/// Whether an attribute is `#[kind…]` or `#[spacetimedb::kind…]`.
fn attribute_named(attribute: &str, kind: &str) -> bool {
    let inner = attribute.trim_start_matches("#[");
    let inner = inner.strip_prefix("spacetimedb::").unwrap_or(inner);
    inner
        .strip_prefix(kind)
        .is_some_and(|rest| rest.starts_with(['(', ']']))
}

/// The comma-separated arguments of an attribute, trimmed.
fn attribute_arguments(attribute: &str) -> Vec<&str> {
    let Some(open) = attribute.find('(') else {
        return Vec::new();
    };
    let inner = attribute[open + 1..].trim_end();
    let inner = inner.strip_suffix(']').unwrap_or(inner);
    let inner = inner.strip_suffix(')').unwrap_or(inner);
    let mut arguments = Vec::new();
    let (mut depth, mut from) = (0usize, 0);
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                arguments.push(inner[from..i].trim());
                from = i + 1;
            }
            _ => {}
        }
    }
    arguments.push(inner[from..].trim());
    arguments
}

/// The end of the Rust item starting at `start`: after the brace that closes
/// its first `{`, or after its `;` when that comes first.
fn rust_item_end(source: &str, start: usize) -> Option<usize> {
    let rest = &source[start..];
    let brace = rest.find('{');
    match (rest.find(';'), brace) {
        (Some(semicolon), Some(brace)) if semicolon < brace => Some(start + semicolon + 1),
        (Some(semicolon), None) => Some(start + semicolon + 1),
        (_, Some(brace)) => closing(source, start + brace),
        _ => None,
    }
}

/// TypeScript definitions in source order. Private tables are left out:
/// clients cannot read them, so there is nothing to ask.
fn typescript_definitions(source: &str) -> Vec<Definition<'_>> {
    let mut found = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let Some((name, value)) = typescript_declaration(source, start, line) else {
            continue;
        };
        let callee = value.split('(').next().unwrap_or("");
        let access = match callee.rsplit('.').next() {
            Some("table") if callee == "table" => Access::Table,
            Some("view") if callee.contains('.') => Access::View,
            Some("reducer") if callee.contains('.') => Access::Reducer,
            _ => continue,
        };
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
        let open = start + (value.as_ptr() as usize - line.as_ptr() as usize) + callee.len();
        let Some(end) = closing(source, open).filter(|_| valid) else {
            continue;
        };
        let text = source[start..end].trim_start();
        if access == Access::Table && !first_argument(text).contains("public: true") {
            continue;
        }
        found.push(Definition {
            access,
            name,
            source: text.into(),
            start_line: crate::analysis::line_of(source, start),
            end_line: crate::analysis::line_of(source, end.saturating_sub(1)),
        });
    }
    found
}

/// The name and value a line declares: `const NAME = value`, or a
/// registration named by its first argument, as in
/// `spacetimedb.reducer('send_message', …)`, whose value is the line itself.
fn typescript_declaration<'a>(
    source: &'a str,
    start: usize,
    line: &'a str,
) -> Option<(&'a str, &'a str)> {
    let declaration = line.trim_start().trim_start_matches("export ");
    if let Some(rest) = declaration.strip_prefix("const ") {
        let (name, value) = rest.split_once('=')?;
        return Some((
            name.split(':').next().unwrap_or("").trim(),
            value.trim_start(),
        ));
    }
    let callee = declaration.split('(').next().unwrap_or("");
    if !(callee.ends_with(".reducer") || callee.ends_with(".view")) {
        return None;
    }
    let opened =
        start + (declaration.as_ptr() as usize - line.as_ptr() as usize) + callee.len() + 1;
    let argument = source[opened.min(source.len())..].trim_start();
    let quote = argument
        .chars()
        .next()
        .filter(|c| matches!(c, '\'' | '"'))?;
    let name = argument[1..].split(quote).next()?;
    Some((name, declaration))
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

/// Columns that hold an identity, or a user, owner, account, member, customer, player or character
/// user id: evidence of whose rows a table holds. TypeScript columns are
/// `name: t.kind()`, Rust fields `name: Type`.
fn user_columns(table: &str) -> Vec<String> {
    let mut columns: Vec<String> = Vec::new();
    for (name, kind) in rust_fields(table)
        .into_iter()
        .chain(typescript_columns(table))
    {
        let lower = name.to_lowercase().replace('_', "");
        let owner = OWNER_WORDS
            .iter()
            .any(|word| lower == *word || lower == format!("{word}id"));
        let identity = kind.eq_ignore_ascii_case("identity");
        if (identity || owner) && !columns.iter().any(|c| c == name) {
            columns.push(name.to_string());
        }
    }
    columns
}

/// Rust struct fields as (name, type): `pub owner: Identity,`.
fn rust_fields(table: &str) -> Vec<(&str, &str)> {
    table
        .lines()
        .filter(|l| !l.contains("t.") && !l.trim_start().starts_with("#["))
        .filter_map(|line| {
            let field = line.trim().trim_start_matches("pub ").trim_end_matches(',');
            let (name, kind) = field.split_once(':')?;
            let name = name.trim();
            let valid = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_');
            valid.then_some((name, kind.trim()))
        })
        .collect()
}

/// TypeScript columns as (name, kind): `owner: t.identity()`.
fn typescript_columns(table: &str) -> Vec<(&str, &str)> {
    table
        .match_indices(": t.")
        .filter_map(|(at, _)| {
            let before = &table[..at];
            let start = before
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map_or(0, |i| i + 1);
            let rest = &table[at + 4..];
            let kind = &rest[..rest
                .find(|c: char| !c.is_alphanumeric())
                .unwrap_or(rest.len())];
            (start < at).then_some((&table[start..at], kind))
        })
        .collect()
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
    let rust = file.path.extension().is_some_and(|e| e == "rs");
    let framework = questions::spacetimedb_framework(version, rust);
    out.rules.insert(ACCESS_CONTROL, 0);
    for definition in definitions(file.path, file.source) {
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
                "columns_naming_users": user_columns(&definition.source),
            });
        } else {
            state[key] = json!({"name": definition.name, "source": definition.source});
            if let Some(table) = scheduled_by(file.source, definition.name) {
                state[key]["scheduled_by"] = json!(table);
            }
            let found = helpers(&definition.source, &lookup);
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
            identity: identity(&[&id, &compact(&definition.source)]),
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

    /// Each definition's kind, name and lines.
    fn kinds<'a>(found: &'a [Definition<'a>]) -> Vec<(&'a Access, &'a str, usize, usize)> {
        found
            .iter()
            .map(|d| (&d.access, d.name, d.start_line, d.end_line))
            .collect()
    }

    #[test]
    fn public_tables_views_and_reducers_are_found_with_their_source() {
        let found = definitions(Path::new("module.ts"), MODULE);
        assert_eq!(
            kinds(&found),
            [
                (&Access::Table, "character", 3, 11),
                (&Access::View, "myCharacters", 15, 17),
                (&Access::Reducer, "renameCharacter", 19, 22),
            ]
        );
        assert!(found[2].source.ends_with("update({ ...row, name })\n})"));
        let registered = "import { schema } from 'spacetimedb/server'\n\nspacetimedb.reducer('send_message', { text: t.string() }, (ctx, { text }) => {\n  ctx.db.message.insert({ sender: ctx.sender, text })\n})\nspacetimedb.reducer(\n  'set_name',\n  { name: t.string() },\n  (ctx, { name }) => {}\n)\n";
        let statements = definitions(Path::new("index.ts"), registered);
        assert_eq!(
            kinds(&statements),
            [
                (&Access::Reducer, "send_message", 3, 5),
                (&Access::Reducer, "set_name", 6, 10)
            ]
        );
        assert_eq!(user_columns(&found[0].source), ["owner", "accountId"]);
        assert!(spacetimedb_module(MODULE));
        assert!(!spacetimedb_module(
            "import { t } from './spacetimedb/server-types'"
        ));
    }

    #[test]
    fn rust_tables_reducers_and_views_are_found_by_attribute() {
        let module = "use spacetimedb::{Identity, ReducerContext, Table};\n\n#[spacetimedb::table(accessor = player, public)]\n#[spacetimedb::table(accessor = logged_out_player)]\n#[derive(Debug, Clone)]\npub struct Player {\n    #[primary_key]\n    identity: Identity,\n    player_id: u32,\n    name: String,\n}\n\n#[table(name = config)]\npub struct Config {\n    world_size: u64,\n}\n\n#[spacetimedb::reducer(init)]\npub fn init(ctx: &ReducerContext) {}\n\n#[spacetimedb::reducer]\npub fn move_all_players(ctx: &ReducerContext, _timer: MoveTimer) -> Result<(), String> {\n    // TODO identity check\n    Ok(())\n}\n\n#[spacetimedb::view(accessor = my_player, public)]\nfn my_player(ctx: &ViewContext) -> Option<Player> {\n    ctx.db.player().identity().find(ctx.sender)\n}\n";
        assert!(spacetimedb_module(module));
        let found = definitions(Path::new("lib.rs"), module);
        assert_eq!(
            kinds(&found),
            [
                (&Access::Table, "player", 3, 11),
                (&Access::Reducer, "move_all_players", 21, 25),
                (&Access::View, "my_player", 27, 30),
            ]
        );
        assert!(found[0].source.starts_with("#[spacetimedb::table(accessor = player, public)]\n#[derive(Debug, Clone)]\npub struct Player {"));
        assert!(!found[0].source.contains("logged_out_player"));
        assert_eq!(user_columns(&found[0].source), ["identity", "player_id"]);
        assert!(!spacetimedb_module(
            "#[table(name = users)]\nstruct User;\n"
        ));
        let scheduled = "#[spacetimedb::table(accessor = move_timer, scheduled(move_all_players))]\npub struct MoveTimer {}\n";
        assert_eq!(
            scheduled_by(scheduled, "move_all_players"),
            Some("#[spacetimedb::table(accessor = move_timer, scheduled(move_all_players))]")
        );
        assert_eq!(scheduled_by(scheduled, "move"), None);
        let typescript = "const timer = table(\n  { name: 'timer', scheduled: 'send_scheduled_message' },\n  { scheduledId: t.u64() }\n)\nconst other = table({ scheduled: () => runMyTimer }, {})\n";
        assert_eq!(
            scheduled_by(typescript, "send_scheduled_message"),
            Some("{ name: 'timer', scheduled: 'send_scheduled_message' },")
        );
        assert!(scheduled_by(typescript, "runMyTimer").is_some());
        assert_eq!(scheduled_by(typescript, "send_scheduled"), None);
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
