//! Access control in SQL: the final state of row-level security policies,
//! SECURITY DEFINER functions and grants across a project's SQL files, read in
//! path order so later migrations replace or drop earlier statements. Each
//! policy is judged with its table and the functions it calls; each definer
//! function and grant alone. Findings sit on the statement that is in force.
use super::{Access, Detail, FileContext, FilePlan, Planned, Presence, Questions, UnitPlan};
use super::{compact, identity, questions};
use crate::{
    analysis::sql::{self, Kind, Statement, short},
    catalog::ACCESS_CONTROL,
    inventory::Input,
    options::CheckArgs,
    schema::Pass,
    token_budget::TokenBudget,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Statements of one project, each with the file that holds it.
#[derive(Default)]
struct State {
    policies: BTreeMap<(String, String), (usize, Statement)>,
    functions: BTreeMap<String, (usize, Statement, bool)>,
    tables: BTreeMap<String, Statement>,
    row_security: BTreeMap<String, bool>,
    grants: Vec<(usize, Statement, Option<String>)>,
    /// Grants and revokes of EXECUTE, by function.
    execute: BTreeMap<String, Vec<Statement>>,
}

/// The directory a file's migrations belong to: above its last `supabase`
/// directory, else above `migrations`, else its own directory.
fn project(path: &Path) -> PathBuf {
    let parts: Vec<_> = path.iter().collect();
    let cut = parts
        .iter()
        .rposition(|p| *p == "supabase")
        .or_else(|| parts.iter().rposition(|p| *p == "migrations"))
        .unwrap_or(parts.len().saturating_sub(1));
    parts[..cut].iter().collect()
}

pub(super) fn plan(
    files: &[(usize, &Input)],
    args: &CheckArgs,
    budget: &TokenBudget,
    plans: &mut BTreeMap<usize, FilePlan>,
    requests: &mut Vec<Planned>,
) {
    let mut projects = BTreeMap::<PathBuf, Vec<(usize, &Input)>>::new();
    for &(owner, input) in files {
        let judged = input.result.role == crate::inventory::SQL;
        plans.insert(
            owner,
            FilePlan {
                path: input.result.path.clone(),
                rules: if judged {
                    BTreeMap::from([(ACCESS_CONTROL, 0)])
                } else {
                    BTreeMap::new()
                },
                units: Vec::new(),
            },
        );
        projects
            .entry(project(&input.result.path))
            .or_default()
            .push((owner, input));
    }
    for mut members in projects.into_values() {
        members.sort_by(|a, b| a.1.result.path.cmp(&b.1.result.path));
        let state = final_state(&members);
        let inputs: BTreeMap<usize, &Input> = members.into_iter().collect();
        let context = |owner: usize| {
            let input = inputs[&owner];
            FileContext {
                owner,
                path: &input.result.path,
                language: "SQL",
                source: input.source.as_deref().unwrap_or(""),
                source_hash: &input.result.source_hash,
                model: args.model(),
                budget,
                framework: None,
            }
        };
        let mut push = |owner: usize, unit: Unit| {
            if inputs[&owner].result.role != crate::inventory::SQL {
                return;
            }
            let file = context(owner);
            let plan = plans.get_mut(&owner).unwrap();
            push_unit(&file, unit, plan, requests);
        };
        for ((table, name), (owner, statement)) in &state.policies {
            push(*owner, policy_unit(&state, table, name, statement));
        }
        for (name, (owner, statement, definer)) in &state.functions {
            if *definer {
                push(*owner, definer_unit(&state, name, statement));
            }
        }
        for (owner, statement, target) in &state.grants {
            push(*owner, grant_unit(&state, statement, target.as_deref()));
        }
    }
}

/// Later statements replace or drop earlier ones with the same name.
fn final_state(members: &[(usize, &Input)]) -> State {
    let mut state = State::default();
    for &(owner, input) in members {
        for statement in sql::statements(input.source.as_deref().unwrap_or("")) {
            match sql::classify(&statement.source) {
                Kind::Policy { name, table } => {
                    state
                        .policies
                        .insert((short(&table).into(), name), (owner, statement));
                }
                Kind::DropPolicy { name, table } => {
                    state.policies.remove(&(short(&table).into(), name));
                }
                Kind::Function { name, definer } => {
                    state
                        .functions
                        .insert(short(&name).into(), (owner, statement, definer));
                }
                Kind::DropFunction { name } => {
                    state.functions.remove(short(&name));
                }
                Kind::Table { name } => {
                    state.tables.insert(short(&name).into(), statement);
                }
                Kind::RowSecurity { table, enabled } => {
                    state.row_security.insert(short(&table).into(), enabled);
                }
                Kind::Grant { target } => {
                    let target = target.map(|t| short(&t).to_string());
                    state.grants.push((owner, statement, target));
                }
                Kind::Execute { function } => {
                    state
                        .execute
                        .entry(short(&function).into())
                        .or_default()
                        .push(statement);
                }
                Kind::Other => {}
            }
        }
    }
    state
}

/// A question's key and its body.
type Asked = (&'static str, fn() -> Value);

struct Unit {
    id: String,
    name: String,
    statement: Statement,
    access: Access,
    state: Value,
    questions: Vec<Asked>,
}

fn policy_unit(state: &State, table: &str, name: &str, statement: &Statement) -> Unit {
    let calls = |function: &str| {
        let text = statement.source.to_lowercase();
        text.match_indices(function).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + function.len()..].trim_start();
            before.is_none_or(|c| !(c.is_alphanumeric() || c == '_')) && after.starts_with('(')
        })
    };
    // A claim read from the token is only as trustworthy as what sets it,
    // such as a custom access token hook that writes `claims`.
    let claims = sql::jwt_claims(&statement.source);
    let sets_claim = |source: &str| {
        let text = source.to_lowercase();
        text.contains("claims")
            && claims
                .iter()
                .any(|c| text.contains(&format!("'{c}'")) || text.contains(&format!("'{{{c}}}'")))
    };
    let functions: Vec<Value> = state
        .functions
        .iter()
        .filter(|(function, (_, s, _))| calls(function) || sets_claim(&s.source))
        .map(|(function, (_, s, _))| json!({"name": function, "source": s.source}))
        .collect();
    Unit {
        id: format!("policy:{table}:{name}"),
        name: name.into(),
        statement: statement.clone(),
        access: Access::Policy {
            table: table.into(),
        },
        state: json!({
            "policy": {"name": name, "source": statement.source},
            "table": {"source": state.tables.get(table).map(|t| t.source.as_str())},
            "functions": functions,
        }),
        questions: vec![
            ("others", questions::policy_others),
            ("editable", questions::policy_editable),
        ],
    }
}

fn definer_unit(state: &State, name: &str, statement: &Statement) -> Unit {
    let privileges: Vec<&str> = state
        .execute
        .get(name)
        .into_iter()
        .flatten()
        .map(|s| s.source.as_str())
        .collect();
    Unit {
        id: format!("function:{name}"),
        name: name.into(),
        statement: statement.clone(),
        access: Access::Definer,
        state: json!({"function": {"name": name, "source": statement.source, "privileges": privileges}}),
        questions: vec![
            ("search_path", questions::definer_search_path),
            ("unchecked", questions::definer_unchecked),
        ],
    }
}

fn grant_unit(state: &State, statement: &Statement, target: Option<&str>) -> Unit {
    let mut evidence = json!({"statement": {"source": statement.source}});
    if let Some(target) = target.filter(|t| state.tables.contains_key(*t)) {
        let enabled = state.row_security.get(target).copied().unwrap_or(false);
        evidence["table"] = json!({"row_level_security": enabled});
    }
    Unit {
        id: format!("grant:{}", statement.start_line),
        name: target.unwrap_or("grant").into(),
        statement: statement.clone(),
        access: Access::Grant,
        state: evidence,
        questions: vec![("broad", questions::grant_broad)],
    }
}

fn push_unit(file: &FileContext<'_>, unit: Unit, plan: &mut FilePlan, requests: &mut Vec<Planned>) {
    let mut questions = Questions::default();
    for (question, body) in &unit.questions {
        questions.ask(
            (*question).into(),
            body(),
            &unit.id,
            ACCESS_CONTROL,
            question,
            Pass::First,
        );
    }
    let mut state = unit.state;
    state["file"] = file.file_state();
    let (request, asked) = file.request("access", state, questions);
    let fits = file.budget.fits(&request);
    let statement = &unit.statement;
    plan.units.push(UnitPlan {
        rule: ACCESS_CONTROL,
        id: unit.id.clone(),
        name: unit.name.clone(),
        presence: if fits {
            Presence::Judged
        } else {
            Presence::NeedsContext
        },
        locations: vec![file.location(statement.start_line, statement.end_line, Some(&unit.name))],
        quote: None,
        lines: statement.end_line + 1 - statement.start_line,
        identity: identity(&[&unit.id, &compact(&statement.source)]),
        detail: Detail::Access(unit.access),
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

#[cfg(test)]
mod tests {
    use super::project;
    use std::path::Path;

    #[test]
    fn migrations_group_by_the_project_that_holds_them() {
        assert_eq!(
            project(Path::new("examples/chat/supabase/migrations/1_init.sql")),
            Path::new("examples/chat")
        );
        assert_eq!(project(Path::new("db/migrations/2.sql")), Path::new("db"));
        assert_eq!(project(Path::new("schema/app.sql")), Path::new("schema"));
    }
}
