//! Access control on SQL and SpacetimeDB modules, and workflows.
use super::*;

const FIRST_MIGRATION: &str = "create table public.notes (id uuid primary key, owner_id uuid not null, body text);\nalter table public.notes enable row level security;\ncreate policy \"read notes\" on public.notes for select using (true);\n";
const SECOND_MIGRATION: &str = "drop policy \"read notes\" on public.notes;\ncreate policy \"read own notes\" on public.notes for select using (owner_id = auth.uid());\ncreate function public.note_count(uid uuid) returns bigint language sql security definer as $$ select count(*) from public.notes where owner_id = uid $$;\ngrant select on public.notes to authenticated;\n";
const WORKFLOW_FILE: &str = "on:\n  pull_request_target:\njobs:\n  greet:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo \"${{ github.event.pull_request.title }}\"\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: make\n";

fn configuration_project() -> (Project, CheckArgs) {
    project_with(
        &[
            ("supabase/migrations/1_init.sql", FIRST_MIGRATION),
            ("supabase/migrations/2_own.sql", SECOND_MIGRATION),
            (".github/workflows/greet.yml", WORKFLOW_FILE),
            ("lib.rs", &function("unrelated")),
        ],
        &[catalog::ACCESS_CONTROL, catalog::WORKFLOWS],
    )
}

#[test]
fn access_units_follow_the_final_state_across_migrations() {
    let (project, options) = configuration_project();
    let (inputs, plan) = planned(&project, &options);
    assert!(
        inputs
            .iter()
            .all(|i| i.result.path.extension().is_some_and(|e| e != "rs")),
        "no application source is collected for these rules"
    );
    let access: Vec<(&str, &str)> = plan
        .requests
        .iter()
        .filter(|p| p.request["jevgate"]["stage"] == "access")
        .map(|p| {
            let state = &p.request["state"];
            let kind = ["policy", "function", "statement"]
                .into_iter()
                .find(|k| state.get(k).is_some())
                .unwrap();
            (kind, state["file"]["path"].as_str().unwrap())
        })
        .collect();
    assert_eq!(
        access,
        [
            ("policy", "supabase/migrations/2_own.sql"),
            ("function", "supabase/migrations/2_own.sql"),
            ("statement", "supabase/migrations/2_own.sql"),
        ],
        "the dropped policy is not judged"
    );
    let policy = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["policy"].is_object())
        .unwrap();
    assert!(
        policy.request["state"]["table"]["source"]
            .as_str()
            .unwrap()
            .starts_with("create table public.notes"),
        "the table from the earlier migration is evidence"
    );
    let grant = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["statement"].is_object())
        .unwrap();
    assert_eq!(grant.request["state"]["table"]["row_level_security"], true);
    let jobs: Vec<&str> = plan
        .requests
        .iter()
        .filter(|p| p.request["jevgate"]["stage"] == "workflows")
        .map(|p| p.request["state"]["job"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        jobs,
        ["greet", "build"],
        "both jobs run on pull_request_target"
    );
    let greet = &plan
        .requests
        .iter()
        .find(|p| p.request["state"]["job"]["name"] == "greet")
        .unwrap()
        .request;
    assert_eq!(
        greet["state"]["expressions"],
        json!(["github.event.pull_request.title"])
    );
    assert_eq!(greet["questions"].as_object().unwrap().len(), 2);
}

#[test]
fn an_undecided_job_is_cleared_by_its_recheck_choices() {
    let (project, mut options) = project_with(
        &[(".github/workflows/greet.yml", WORKFLOW_FILE)],
        &[catalog::WORKFLOWS],
    );
    options.refresh = true;
    let status = |outside: &str| {
        let mut eval = scripted(0);
        eval.overrides = vec![("outside", noul_at(0.5)), ("untrusted", noul_at(0.5))];
        eval.recheck_level = Some(0);
        eval.recheck_overrides = vec![
            ("outside_source", choice_of(outside, &["e0", "none"])),
            (
                "pull_request_code",
                choice_of("base", &["base", "pull_request", "none"]),
            ),
        ];
        let report = run(&project, &options, &mut eval);
        report.files[0].dimensions[catalog::WORKFLOWS]
            .status
            .clone()
    };
    assert_eq!(status("none"), Status::Clear);
    assert_eq!(
        status("e0"),
        Status::Uncertain,
        "a Choice that names the title only clears nothing"
    );
}

#[test]
fn access_control_judges_migrations_beside_the_code_rules() {
    let (project, mut options) = configuration_project();
    options.rules.push(catalog::INJECTION.into());
    let (inputs, plan) = planned(&project, &options);
    let migration = inputs
        .iter()
        .find(|i| i.result.path.ends_with("2_own.sql"))
        .unwrap();
    assert_eq!(migration.result.role, crate::inventory::SQL);
    assert_eq!(
        plan.requests
            .iter()
            .filter(|p| p.request["jevgate"]["stage"] == "access")
            .count(),
        3,
        "the code rules' walk does not hide migrations from access control"
    );
}

#[test]
fn an_unchecked_definer_is_a_review_and_an_open_search_path_a_consider() {
    let (project, options) = configuration_project();
    let report = run_with_nouls(
        &project,
        &options,
        &[("search_path", 0.9), ("outside", 0.95)],
    );
    let file = |name: &str| {
        report
            .files
            .iter()
            .find(|f| f.path.ends_with(name))
            .unwrap()
    };
    let definer = &file("2_own.sql").findings[0];
    assert_eq!(definer.strength, Strength::Consider);
    assert_eq!(definer.rule, "security/access-control");
    assert_eq!(
        definer.category.as_deref(),
        Some("CWE-426 untrusted search path")
    );
    assert!(
        definer
            .message
            .starts_with("SECURITY DEFINER function `note_count`")
    );
    assert_eq!(file("1_init.sql").status, Status::NotApplicable);
    let job = &file("greet.yml").findings[0];
    assert_eq!(job.strength, Strength::Review);
    assert_eq!(job.category.as_deref(), Some("CWE-78 command injection"));
    assert!(
        job.message
            .contains("`${{ github.event.pull_request.title }}`"),
        "{}",
        job.message
    );
    assert_eq!(job.locations[0].start_line, 4);
}

const STDB_TABLES: &str = "import { table, t } from 'spacetimedb/server'\n\nexport const character = table(\n  { public: true },\n  {\n    id: t.u64().primaryKey(),\n    owner: t.identity(),\n    name: t.string(),\n  },\n)\n\nexport const account = table({ public: false }, { owner: t.identity().primaryKey() })\n";
const STDB_COMMANDS: &str = "import { t } from 'spacetimedb/server'\nimport { database } from './schema'\nimport { ownedCharacter } from './owned'\n\nexport const renameCharacter = database.reducer({ characterId: t.u64(), name: t.string() }, (ctx, { characterId, name }) => {\n  const row = ownedCharacter(ctx, characterId)\n  ctx.db.character.id.update({ ...row, name })\n})\n\nexport const myCharacters = database.view({ public: true }, t.array(character.rowType), (ctx) => {\n  return [...ctx.db.character.owner.filter(ctx.sender)]\n})\n";
const STDB_OWNED: &str = "export function ownedCharacter(ctx, id) {\n  const row = ctx.db.character.id.find(id)\n  if (!row || !row.owner.isEqual(ctx.sender)) throw new Error('NOT_OWNER')\n  return row\n}\n";

fn spacetimedb_project() -> (Project, CheckArgs) {
    project_with(
        &[
            (
                "server/package.json",
                "{\"dependencies\": {\"spacetimedb\": \"^2.10.0\"}}",
            ),
            ("server/src/tables.ts", STDB_TABLES),
            ("server/src/commands.ts", STDB_COMMANDS),
            ("server/src/owned.ts", STDB_OWNED),
            (
                "web/src/owned.ts",
                "export function ownedCharacter(id) {\n  return fetch(`/characters/${id}`)\n}\n",
            ),
        ],
        &[catalog::ACCESS_CONTROL],
    )
}

#[test]
fn spacetimedb_tables_views_and_reducers_are_judged_with_helpers_and_the_version() {
    let (project, options) = spacetimedb_project();
    let (_, plan) = planned(&project, &options);
    let units: Vec<(&str, &Detail)> = plan
        .files
        .values()
        .flat_map(|f| f.units.iter().map(|u| (u.name.as_str(), &u.detail)))
        .collect();
    assert_eq!(units.len(), 3, "the private table is not asked about");
    let reducer = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["reducer"].is_object())
        .unwrap();
    let state = &reducer.request["state"];
    assert_eq!(state["helpers"][0]["name"], "ownedCharacter");
    assert!(
        state["helpers"][0]["source"]
            .as_str()
            .unwrap()
            .contains("ctx.sender"),
        "the helper nearest the module, not the web client's"
    );
    let note = reducer.request["questions"]["reach"]["instructions"]["note"]
        .as_str()
        .unwrap();
    assert!(
        note.contains("SpacetimeDB 2.10.0")
            && note.contains("Scheduled reducers (named by a table's `scheduled` option, shown as `scheduled_by`) are private")
    );
    assert_eq!(
        reducer.request["jevgate"]["sources"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let table = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["table"].is_object())
        .unwrap();
    assert_eq!(
        table.request["state"]["table"]["columns_naming_users"],
        json!(["owner"])
    );
    // Acceptable levels clear; a literal check at review raises the reducer,
    // while a public table is at most a consider.
    let report = run(&project, &options, &mut scripted(0));
    assert!(
        report
            .files
            .iter()
            .filter(|f| f.path.starts_with("server/src"))
            .all(|f| f.status == Status::Clear || f.status == Status::NotApplicable),
        "{:?}",
        report
            .files
            .iter()
            .map(|f| (&f.path, &f.status))
            .collect::<Vec<_>>()
    );
    let mut options = options;
    options.refresh = true;
    let report = run_with_nouls(
        &project,
        &options,
        &[("argument_rows", 0.9), ("exposed", 0.95)],
    );
    let findings: Vec<&crate::schema::Finding> =
        report.files.iter().flat_map(|f| &f.findings).collect();
    let reducer = findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("renameCharacter"))
        .unwrap();
    assert_eq!(reducer.strength, Strength::Review);
    assert!(
        reducer
            .message
            .starts_with("Reducer `renameCharacter` reads or changes a row its arguments choose"),
        "{}",
        reducer.message
    );
    assert_eq!(
        reducer.category.as_deref(),
        Some("CWE-639 authorization through a user-controlled key")
    );
    let table = findings
        .iter()
        .find(|f| f.symbol.as_deref() == Some("character"))
        .unwrap();
    assert_eq!(table.strength, Strength::Consider);
    assert!(
        table
            .message
            .starts_with("Public table `character` likely lets every client read")
    );
}

#[test]
fn a_definer_is_sent_with_the_project_functions_it_calls_and_uninstall_scripts_are_left_out() {
    let install = "create function public.is_claims_admin() returns bool language plpgsql as $$ begin return coalesce(auth.jwt() ->> 'claims_admin', 'false')::bool; end; $$;\ncreate function public.set_claim(uid uuid, claim text, value jsonb) returns text language plpgsql security definer set search_path = public as $$ begin if not is_claims_admin() then return 'error: access denied'; end if; update auth.users set raw_app_meta_data = raw_app_meta_data || json_build_object(claim, value)::jsonb where id = uid; return 'OK'; end; $$;\n";
    let uninstall = "drop function is_claims_admin;\ndrop function set_claim;\n";
    let (project, options) = project_with(
        &[("install.sql", install), ("uninstall.sql", uninstall)],
        &[catalog::ACCESS_CONTROL],
    );
    let (_, plan) = planned(&project, &options);
    let request = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["function"]["name"] == "set_claim")
        .expect("the definer is judged although uninstall.sql drops it");
    assert_eq!(
        request.request["state"]["functions"][0]["name"],
        "is_claims_admin"
    );
}
