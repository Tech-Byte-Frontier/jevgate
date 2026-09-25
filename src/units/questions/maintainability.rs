//! Questions of the maintainability rules: splitting and flattening
//! functions, file outlines and their kinds, hardcoded values and copies.
use super::{EVIDENCE, choose_id, noul, score};
use serde_json::{Map, Value, json};

/// Asks whether splitting would help a reader, not how many tasks the function
/// performs: counting tasks read multi-step functions literally, and most
/// functions it called multi-task were fine as written.
pub fn function_split(path: &str, callees: bool) -> Value {
    let note = if callees {
        format!("`callees` lists the signatures of functions it calls. {EVIDENCE}")
    } else {
        EVIDENCE.to_string()
    };
    json!({
        "type": "score",
        "instructions": {
            "question": format!("Would splitting the function in `{path}` into smaller named functions make it easier to understand?"),
            "note": note,
        },
        "criteria": [
            "No. It reads as one job: its steps are short, already call named functions, or belong together, such as checks before a write, one transaction, or one component and its markup.",
            "Slightly. One block could be named as a helper, but the function is readable as it is.",
            "Yes. It mixes separate jobs in long blocks, so a reader must keep unrelated details in mind at once.",
        ],
    })
}

/// Asked only when the parser finds deep nesting or a long branch chain.
pub fn function_flatten(path: &str) -> Value {
    score(
        format!(
            "Would guard clauses, early returns or a lookup table make the branching in `{path}` easier to follow?"
        ),
        "",
        [
            "No. The branching reads clearly as written.",
            "Slightly. One condition could return early, but the flow is easy to follow.",
            "Yes. Nested or repeated branches hide the main path, and flattening them would make it clear.",
        ],
    )
}

/// Asked only after the split question raised a finding, to locate it.
pub fn function_block(blocks: &[String]) -> Value {
    choose_id(
        "Which block in `function.blocks` would be most useful as its own named function?",
        format!(
            "`function.blocks` holds the body in order. Options are the `id` values in `function.blocks`. {EVIDENCE}"
        ),
        blocks,
        "No single block would be clearer as its own function.",
    )
}

/// Whether a file would be easier to navigate as several modules. `tests`
/// asks about a test file's cases. Sizes are evidence, never a limit: "areas"
/// and "kinds of records" as the concern flagged one resource's routes and
/// models, so the concern names separate features, rules or subjects.
pub fn outline_split(tests: bool, source: bool) -> Value {
    let (question, listed, criteria) = if tests {
        (
            "Would moving some of the tests in `members` into a separate test file make this file easier to navigate and maintain?",
            "Members are test cases, with the suite that encloses each one and the functions under test it calls (`subjects`), and the helpers they share; bodies are not included. `groups` lists members that share a suite, a subject or a helper.",
            [
                "No. The tests cover one subject, such as one module, one feature, one endpoint group or one component, with the helpers they share, even when the file is long.",
                "Slightly. A few tests could live elsewhere, but the file is easy to navigate as it is.",
                "Yes. The file tests several subjects that a reader works on separately, such as unrelated modules, features or commands, and each subject's tests would be easier to find in their own file.",
            ],
        )
    } else {
        (
            "Would moving some of the members in `members` into a separate module make this file easier to navigate and maintain?",
            "Members are listed by signature and the names they call; bodies are not included. `groups` lists members that call each other or share types. `used_by` names a few other files that call a member. A long file that serves one feature is fine.",
            [
                "No. The members serve one feature, resource or job, such as one algorithm, one type and its helpers, the routes and queries of one resource, one screen and its parts, or a few small related helpers.",
                "Slightly. One small set of members could live elsewhere, but the file is easy to navigate as it is.",
                "Yes. The file holds several features or subsystems, such as separate product features, separate rules or separate integrations, that are read and changed apart, and each would be easier to find in its own module.",
            ],
        )
    };
    let sizes = "`lines` is each member's length and `file.lines` the whole file's.";
    let note = if source {
        format!("`file.source` holds the file. {listed} {sizes}")
    } else {
        format!("{listed} {sizes}")
    };
    score(question.into(), &note, criteria)
}

pub fn outline_module(tests: bool, groups: &[String]) -> Value {
    choose_id(
        if tests {
            "Which group in `groups` would be most useful as its own test file?"
        } else {
            "Which group in `groups` would be most useful as its own module?"
        },
        "Options are the `id` values in `groups`.".into(),
        groups,
        if tests {
            "No group would be more useful as its own test file."
        } else {
            "No group would be more useful as its own module."
        },
    )
}

/// Kinds of files whose members serve several features; every other kind
/// serves one. Asked with the recheck: when the split Score stays undecided,
/// the kind decides, since naming what a file holds was decisive where
/// weighing a split was not.
pub const SEVERAL_KINDS: [&str; 2] = ["per_feature", "several"];

/// What the members of a file hold, as a Choice among kinds of files.
pub fn outline_kind(tests: bool) -> Value {
    let kinds: &[(&str, &str)] = if tests {
        &[
            (
                "one_subject",
                "Tests of one module, feature, endpoint group or component, with shared helpers.",
            ),
            (
                "per_feature",
                "Tests of one kind written out for each of several features or commands.",
            ),
            (
                "several",
                "Tests of several unrelated modules, features or subsystems.",
            ),
        ]
    } else {
        &[
            (
                "algorithm",
                "One algorithm, process or pipeline stage and the helpers it uses.",
            ),
            (
                "type",
                "One type, class or object and its methods, or one small family of related types.",
            ),
            (
                "resource",
                "One resource or domain entity: its routes, handlers, queries or data access.",
            ),
            ("component", "One screen, component or view and its parts."),
            (
                "definitions",
                "Definitions of one kind for one area: data types, schemas, constants, configuration or options.",
            ),
            ("helpers", "A few small, related helper functions."),
            (
                "coordination",
                "Coordination: code that runs the steps and calls other modules that own each feature.",
            ),
            (
                "per_feature",
                "The same kind of code written out for each of several features, rules or cases, such as each feature's messages, checks or handlers.",
            ),
            (
                "several",
                "Several unrelated features, integrations or subsystems.",
            ),
        ]
    };
    json!({
        "type": "choice",
        "instructions": {
            "question": "Which best describes what the members in `members` hold?",
            "note": format!("`file.source` holds the file. {EVIDENCE}"),
        },
        "criteria": kinds.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_, _>>(),
    })
}

/// Asked only after a hardcoded-value finding, to name the value it is about.
pub fn hardcoded_value(ids: &[String]) -> Value {
    choose_id(
        "Which value in `function.values` most needs to come from configuration, get a descriptive name, or be read from data instead of being written in `function.source`?",
        format!("Options are the `id` values in `function.values`. {EVIDENCE}"),
        ids,
        "No single value stands out.",
    )
}

/// Whether a value fixed in code changes between environments. `values` names
/// the list of candidate values; `code` describes the code that uses them.
/// Criteria name what is not environment-specific (the program's own routes,
/// project-relative paths, public addresses): examples of "URL" and "path"
/// alone were read literally and flagged routes and repository files.
pub fn hardcoded_environment(values: &str, code: &str) -> Value {
    score(
        format!(
            "Would a value in `{values}` need to change when {code} runs in another environment, such as another server, account or machine?"
        ),
        "",
        [
            "No. Every value stays the same in every environment: the program's own routes and file names, paths relative to the project, public addresses such as documentation links, messages and formats.",
            "Slightly. A value is a default for local development, such as localhost, that configuration already overrides.",
            "Yes. A value names something outside the program that differs between environments, such as a specific server address, database, account, or an absolute path on one machine, fixed in the code or used as its default.",
        ],
    )
}

pub fn hardcoded_magic(values: &str, code: &str) -> Value {
    score(
        format!(
            "Would giving a value in `{values}` a descriptive name make `{code}` easier to understand?"
        ),
        "",
        [
            "No. Each value explains itself where it is used, such as a message, format, key, or a count its context makes clear.",
            "Slightly. One value could be named, but its meaning is clear from the code around it.",
            "Yes. A reader must guess what a number or string means or why it has that value, or the same value repeats.",
        ],
    )
}

pub fn hardcoded_special(code: &str) -> Value {
    noul(
        format!(
            "Does `{code}` treat one specific user, account, tenant, record or name differently from the rest?"
        ),
        "It checks for, or holds a table of, particular users, accounts, tenants, records or items by their literal identifiers, instead of reading them from data or configuration.",
        "It treats every identity alike, or compares against values that are part of the program's own rules, such as states, roles or commands.",
    )
}

/// Asked only when a hardcoded-value question stayed undecided: whether every
/// value is one of the kinds its criteria already call acceptable. It can only
/// clear. Undecided answers were mostly field names, messages, protocol codes
/// and the program's own identifiers; re-asking whether each value needs a
/// name added false findings instead. The unnamed-value check names the kinds
/// in the question and leaves every other number undecided: asked whether
/// each value "explains itself", it cleared no undecided unit, not even one
/// holding only field names.
pub fn hardcoded_benign(question: &str, values: &str, code: &str) -> Value {
    match question {
        "environment" => noul(
            format!(
                "Is every value in `{values}` of a kind that stays the same in every environment?"
            ),
            "Every value is a message, a format, a field, key or column name, an environment variable name, one of the program's own routes or file names, a path relative to the project, or a public address such as a documentation link.",
            "At least one value names a specific server, host, port, database, account or credential, or is an absolute path on one machine.",
        ),
        "magic" => noul(
            format!(
                "Is every value in `{values}` a string or code that reads for itself, such as a message, format, key, field or protocol name, or status code?"
            ),
            "Yes. Every value is a message, a format or template, a key, field, column or header name, a name or argument defined by a protocol, tool, grammar or file format, one of the program's own names such as a state, role or command, or an HTTP status or error code.",
            "No. At least one value is another number, or a string whose meaning or reason a reader must guess.",
        ),
        _ => noul(
            format!(
                "Is every literal identifier that `{code}` compares against or looks up a name the program defines or a protocol uses?"
            ),
            "Each is part of the program's own vocabulary or a protocol, such as a state, command, role, key, service or credential name, not a particular user, account, tenant, customer or record.",
            "At least one names a particular user, account, tenant, customer or record.",
        ),
    }
}

pub fn duplicate_same(recheck: bool) -> Value {
    let note = if recheck {
        "`site_a.function_source` and `site_b.function_source` hold the enclosing functions."
    } else {
        "`site_a.function` and `site_b.function` name the enclosing functions."
    };
    score(
        "Do `site_a.source` and `site_b.source` perform the same steps for the same purpose?"
            .into(),
        note,
        [
            "Different work that only looks alike.",
            "Related steps; a person should decide whether they belong together.",
            "The same steps for the same purpose. One shared implementation would serve both.",
        ],
    )
}

pub fn duplicate_only_differences() -> Value {
    noul(
        "Do `site_a.source` and `site_b.source` differ only in the names and values listed in `differences`?".into(),
        "Apart from the listed names and values, the two sites are the same.",
        "The sites also differ in other ways that matter.",
    )
}

/// "Separate test cases" in the old wording did not cover cases written out
/// one after another inside a single test.
pub fn duplicate_required() -> Value {
    json!({
        "type": "noul",
        "instructions": {
            "question": "Does each of `site_a.source` and `site_b.source` spell out its own case, so that repeating the steps is how the cases are written?",
            "note": format!("A case can be one scenario of a test, one input of a table of checks or one attempt of a retry. {EVIDENCE}"),
        },
        "criteria": {
            "true": "Each copy sets up or checks a different case, and the differing names and values are the point of each copy.",
            "false": "The copies implement the same work twice, and one shared function, fixture or helper could replace them.",
        },
    })
}
