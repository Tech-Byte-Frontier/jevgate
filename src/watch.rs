use super::{
    evaluate::{Session, snapshot},
    inventory::{self, Input},
    options::Format,
    output,
    schema::Report,
};
use anyhow::Result;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn stop_watcher(
    session: &Session<'_>,
    report: &mut Report,
    message: impl Into<String>,
    error: anyhow::Error,
) -> Result<()> {
    report.errors.push(message.into());
    report.watcher_pid = None;
    report.update_status();
    session.publish(report)?;
    Err(error)
}

/// Evaluate a debounced change, compare it with the last settled snapshot, gate
/// and publish it. A failure stops the watcher with its reason recorded.
fn settle(
    session: &mut Session<'_>,
    inputs: &[Input],
    report: &mut Report,
    baseline: &Report,
) -> std::result::Result<(), Result<()>> {
    if let Err(error) = session.evaluate(inputs, report) {
        return Err(stop_watcher(
            session,
            report,
            "Watcher stopped during evaluation",
            error,
        ));
    }
    super::changes::compare(Some(baseline), report);
    if let Err(error) = crate::gate::settle(&session.context.root, report, session.args) {
        return Err(stop_watcher(session, report, error.to_string(), error));
    }
    report.settled = true;
    session.publish(report).map_err(Err)
}

pub fn run(
    session: &mut Session<'_>,
    scope: Vec<PathBuf>,
    mut inputs: Vec<Input>,
    mut report: Report,
) -> Result<()> {
    let mut baseline = report.clone();
    let mut fingerprint = inventory::fingerprint(&inputs);
    let policy = policy_fingerprint(&session.context.root);
    let mut changed_at = None;
    let mut previous = super::evaluate::previous_judgments(Some(&report), false);
    loop {
        wait_for_poll(session, &mut report)?;
        if policy_fingerprint(&session.context.root) != policy {
            return stop_watcher(
                session,
                &mut report,
                "Review configuration, custom questions or root ignore rules changed; restart the watcher to apply them",
                anyhow::anyhow!("Review policy changed; watcher stopped"),
            );
        }
        let current = match inventory::collect(session.args, session.context, &scope) {
            Ok(current) => current,
            Err(error) => {
                return stop_watcher(session, &mut report, error.to_string(), error);
            }
        };
        let current_fingerprint = inventory::fingerprint(&current);
        if fingerprint != current_fingerprint {
            fingerprint = current_fingerprint;
            inputs = current;
            changed_at = Some(Instant::now());
            report = snapshot(
                &inputs,
                &previous,
                session.args,
                super::evaluate::SnapshotContext {
                    root: &session.context.root,
                    generation: report.generation + 1,
                    requests: session.requests,
                },
            );
            // Invalidate changed/deleted files immediately, before the debounce or API request.
            session.publish(&report)?;
        }
        if changed_at
            .is_some_and(|time| time.elapsed() >= Duration::from_millis(session.args.debounce_ms))
        {
            if let Err(stopped) = settle(session, &inputs, &mut report, &baseline) {
                return stopped;
            }
            baseline = report.clone();
            fingerprint = inventory::fingerprint(&inputs);
            previous = super::evaluate::previous_judgments(Some(&report), false);
            if session.args.output_format() != Format::Jsonl {
                output::emit(&report, session.args)?;
            }
            changed_at = None;
        }
    }
}

fn check_running(session: &Session<'_>, report: &mut Report) -> Result<()> {
    if let Err(error) = crate::cancellation::check() {
        return stop_watcher(
            session,
            report,
            "Watcher stopped; this snapshot will no longer track saves",
            error.into(),
        );
    }
    Ok(())
}

fn wait_for_poll(session: &Session<'_>, report: &mut Report) -> Result<()> {
    check_running(session, report)?;
    let next_poll = Instant::now() + Duration::from_millis(session.args.poll_ms);
    while let Some(remaining) = next_poll.checked_duration_since(Instant::now()) {
        std::thread::sleep(remaining.min(Duration::from_secs(1)));
        check_running(session, report)?;
    }
    Ok(())
}

/// A hash of what sets the review policy and the upload boundary:
/// jevgate.toml, the root ignore rules, and each custom question file with
/// its name, which is its id. The configuration is read once per process,
/// so a watcher stops when any of them changes rather than judging with
/// questions that are no longer the repository's.
fn policy_fingerprint(root: &std::path::Path) -> String {
    let hashed = |path: &std::path::Path| {
        std::fs::read(path)
            .map(|b| super::schema::hash(&b))
            .unwrap_or_else(|_| "absent".into())
    };
    let mut values: Vec<String> = ["jevgate.toml", ".gitignore"]
        .iter()
        .map(|p| hashed(&root.join(p)))
        .collect();
    let questions = crate::custom::question_files(&crate::custom::directory(root));
    values.extend(questions.unwrap_or_default().iter().map(|path| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        format!("{name}:{}", hashed(path))
    }));
    super::schema::hash(&serde_json::to_vec(&values).expect("serializable policy"))
}

#[cfg(test)]
mod tests {
    use super::policy_fingerprint;

    #[test]
    fn a_question_file_added_edited_or_renamed_changes_the_policy() {
        let project = crate::tests::Project::new();
        project.write("jevgate.toml", "");
        let before = policy_fingerprint(&project.0);
        let question = "question = \"Does this function log a body?\"\nunit = \"function\"\n";
        project.write(".jevgate/questions/body-logs.toml", question);
        let added = policy_fingerprint(&project.0);
        assert_ne!(added, before);
        project.write(".jevgate/cache/answer.json", "{}");
        assert_eq!(
            policy_fingerprint(&project.0),
            added,
            "the cache is not policy"
        );
        project.write(
            ".jevgate/questions/body-logs.toml",
            &question.replace("body", "request body"),
        );
        let edited = policy_fingerprint(&project.0);
        assert_ne!(edited, added);
        std::fs::rename(
            project.0.join(".jevgate/questions/body-logs.toml"),
            project.0.join(".jevgate/questions/logs.toml"),
        )
        .unwrap();
        assert_ne!(
            policy_fingerprint(&project.0),
            edited,
            "a new name is a new id"
        );
    }
}
