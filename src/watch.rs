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
    let mut previous = report
        .files
        .iter()
        .map(|f| (f.path.clone(), f.clone()))
        .collect();
    loop {
        wait_for_poll(session, &mut report)?;
        if policy_fingerprint(&session.context.root) != policy {
            report.watcher_pid = None;
            report.errors.push("Review configuration or root ignore rules changed; restart the watcher to apply the new upload boundary".into());
            report.update_status();
            session.publish(&report)?;
            anyhow::bail!("Review policy changed; watcher stopped");
        }
        let current = match inventory::collect(session.args, session.context, &scope) {
            Ok(current) => current,
            Err(error) => {
                report.errors.push(error.to_string());
                report.watcher_pid = None;
                report.update_status();
                session.publish(&report)?;
                return Err(error);
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
            if let Err(error) = session.evaluate(&inputs, &mut report) {
                report
                    .errors
                    .push("Watcher stopped during evaluation".into());
                report.watcher_pid = None;
                report.update_status();
                session.publish(&report)?;
                return Err(error);
            }
            super::changes::compare(Some(&baseline), &mut report);
            report.settled = true;
            session.publish(&report)?;
            baseline = report.clone();
            fingerprint = inventory::fingerprint(&inputs);
            previous = report
                .files
                .iter()
                .map(|f| (f.path.clone(), f.clone()))
                .collect();
            if session.args.output_format() != Format::Jsonl {
                output::emit(&report, session.args.output_format())?;
            }
            changed_at = None;
        }
    }
}

fn check_running(session: &Session<'_>, report: &mut Report) -> Result<()> {
    if let Err(error) = crate::cancellation::check() {
        report.watcher_pid = None;
        report
            .errors
            .push("Watcher stopped; this snapshot will no longer track saves".into());
        report.update_status();
        session.publish(report)?;
        return Err(error.into());
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

fn policy_fingerprint(root: &std::path::Path) -> String {
    let values: Vec<_> = ["jevgate.toml", ".gitignore"]
        .iter()
        .map(|p| {
            std::fs::read(root.join(p))
                .map(|b| super::schema::hash(&b))
                .unwrap_or_else(|_| "absent".into())
        })
        .collect();
    super::schema::hash(&serde_json::to_vec(&values).expect("serializable policy"))
}
