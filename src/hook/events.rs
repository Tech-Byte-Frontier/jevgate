//! What each event does in its repository: a session's or turn's start
//! records a snapshot of the working tree, an edit is checked and its
//! findings go to the agent, and a stop is checked against the turn's start
//! and blocked while findings fail the gate.
use super::{
    Host,
    agents::{self, Event, Kind, Reply},
    review::{self, Checked, Flagged},
    text,
    turn::{self, Turn},
};
use crate::config;
use anyhow::{Context, Result};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

/// A failure told to the person and, where the event carries context, to
/// the agent. Nothing blocks.
pub(super) fn failed(event: &Event, what: &str, reason: &str) -> Reply {
    Reply {
        block: false,
        agent: agents::has_context(event.agent, event.kind)
            .then(|| text::failed_agent(what, reason)),
        user: Some(text::failed_user(what, reason)),
    }
}

/// One event in its repository.
pub(super) struct Hook<'a> {
    event: &'a Event,
    host: &'a Host,
    deadline: Instant,
    /// The session's directory and the repository around it.
    cwd: PathBuf,
    root: PathBuf,
    /// The repository's Git index, which snapshots start from.
    index: PathBuf,
}

impl<'a> Hook<'a> {
    /// The repository of the event's directory; it must be a Git work tree,
    /// so no state is written into a directory that is not one.
    pub(super) fn open(event: &'a Event, host: &'a Host, deadline: Instant) -> Result<Self> {
        let cwd = event.cwd.as_deref().unwrap_or(&host.cwd);
        let cwd = cwd
            .canonicalize()
            .with_context(|| format!("Cannot open the session's directory {}", cwd.display()))?;
        let root = config::repository_root(&cwd);
        let index = crate::revision::index_file(&root).with_context(|| {
            format!(
                "{} is not in a Git repository (or Git cannot run), so JevGate cannot tell what a turn changed",
                root.display()
            )
        })?;
        Ok(Self {
            event,
            host,
            deadline,
            cwd,
            root,
            index,
        })
    }

    pub(super) fn handle(&self) -> Reply {
        match self.event.kind {
            Kind::SessionStart => self.session_start(),
            Kind::TurnStart => self.turn_start(),
            Kind::AfterEdit => self.after_edit(),
            Kind::Stop => self.stop(),
            Kind::Other => Reply::default(),
        }
    }

    fn snapshot(&self) -> Result<String> {
        turn::snapshot(&self.root, &self.index, &self.event.session, self.deadline)
    }

    fn load(&self) -> Option<Turn> {
        turn::load(&self.root, &self.event.session)
    }

    /// A session's first turn begins when it starts, in case the agent sends
    /// no turn-start event for its first prompt. A session that has a turn
    /// keeps it: Claude Code and Codex also start a session after compacting
    /// one in the middle of a turn.
    fn session_start(&self) -> Reply {
        match self.load() {
            Some(turn) => self.context(turn, None),
            None => self.begin(None),
        }
    }

    /// A new turn begins, unless the prompt is this turn's block reason sent
    /// back (Gemini CLI and Cursor continue a blocked turn that way).
    fn turn_start(&self) -> Reply {
        let previous = self.load();
        match previous {
            Some(turn) if turn.continued_by(&self.event.prompt) => self.context(turn, None),
            _ => {
                turn::prune(&self.root);
                self.begin(previous.and_then(|turn| turn.notice))
            }
        }
    }

    /// Record a turn that begins now, owing the agent `notice`.
    fn begin(&self, notice: Option<String>) -> Reply {
        let recorded = self.snapshot().and_then(|tree| {
            let turn = Turn::begin(&self.event.session, tree, notice);
            turn::save(&self.root, &turn).map(|()| turn)
        });
        match recorded {
            Ok(turn) => self.context(turn, None),
            Err(error) => failed(self.event, "this turn", &format!("{error:#}")),
        }
    }

    /// `context` for the agent, after the notice `turn` owes it when this
    /// event can carry one; the turn is saved without it once given.
    fn context(&self, mut turn: Turn, context: Option<String>) -> Reply {
        let notice = turn
            .notice
            .take_if(|_| agents::has_context(self.event.agent, self.event.kind));
        if notice.is_some() {
            let _ = turn::save(&self.root, &turn);
        }
        let agent = [notice, context].into_iter().flatten().collect::<Vec<_>>();
        Reply {
            block: false,
            agent: (!agent.is_empty()).then(|| agent.join("\n\n")),
            user: None,
        }
    }

    /// The edited files inside the repository, as absolute paths; files
    /// elsewhere, or no longer there, are not the repository's to check.
    fn edited(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self
            .event
            .files
            .iter()
            .filter_map(|file| self.cwd.join(file).canonicalize().ok())
            .filter(|file| file.starts_with(&self.root) && file.is_file())
            .collect();
        files.sort();
        files.dedup();
        files
    }

    /// Check the edited files since the turn began (whole, without a turn)
    /// and tell the agent their findings. Nothing blocks.
    fn after_edit(&self) -> Reply {
        let turn = self.load();
        let files = self.edited();
        if files.is_empty() {
            return turn.map_or_else(Reply::default, |turn| self.context(turn, None));
        }
        let shown = relative(&self.root, &files);
        let named = text::named(&shown);
        let trees = match &turn {
            Some(turn) => match self.snapshot() {
                Ok(now) => Some((turn.tree.clone(), now)),
                Err(error) => return failed(self.event, &named, &format!("{error:#}")),
            },
            None => None,
        };
        let checked = match self.check(review::Scope {
            trees,
            paths: files,
        }) {
            Ok(checked) => checked,
            Err(reason) => return failed(self.event, &named, &reason),
        };
        let Some(mut turn) = turn else {
            let guards: Vec<_> = checked.guards.iter().collect();
            return Reply {
                agent: text::after_edit(&shown, &checked.flagged, &[], &guards),
                ..Reply::default()
            };
        };
        let (known, new): (Vec<_>, Vec<_>) = checked
            .flagged
            .into_iter()
            .partition(|f| turn.reported.contains(&f.finding.fingerprint));
        let guards: Vec<_> = checked
            .guards
            .iter()
            .filter(|g| !turn.reported.contains(&g.id))
            .collect();
        let context = text::after_edit(&shown, &new, &known, &guards);
        let ids = new.iter().map(|f| f.finding.fingerprint.as_str());
        if turn.report(ids.chain(guards.iter().map(|g| g.id.as_str()))) {
            let _ = turn::save(&self.root, &turn);
        }
        self.context(turn, context)
    }

    /// Check what changed since the turn began, then decide whether the
    /// agent may stop.
    fn stop(&self) -> Reply {
        if !self.event.completed {
            return Reply::default();
        }
        let Some(turn) = self.load() else {
            return self.first_stop();
        };
        let now = match self.snapshot() {
            Ok(now) => now,
            Err(error) => return self.unchecked(turn, &format!("{error:#}")),
        };
        match self.turn_findings(&turn, &now) {
            Ok(checked) => self.decide(turn, now, checked),
            Err(reason) => self.unchecked(turn, &reason),
        }
    }

    /// A stop with no record of the turn's start: one is recorded, so the
    /// next turn is checked, and the person is told this one was not.
    fn first_stop(&self) -> Reply {
        let reply = self.begin(None);
        Reply {
            user: reply.user.or(Some(text::UNCHECKED_TURN.into())),
            ..reply
        }
    }

    /// The findings and guards in what changed from the turn's start to `now`.
    fn turn_findings(&self, turn: &Turn, now: &str) -> Result<Checked, String> {
        if now == turn.tree {
            return Ok(Checked::default());
        }
        self.check(review::Scope {
            trees: Some((turn.tree.clone(), now.to_string())),
            paths: Vec::new(),
        })
    }

    /// Block while findings fail the gate: at most three times a turn, and
    /// not again when nothing changed since the last block. A stop that is
    /// not blocked ends the turn. The person hears of the turn's guards.
    fn decide(&self, turn: Turn, now: String, checked: Checked) -> Reply {
        let blocks = if self.event.continued { turn.blocks } else { 0 };
        let guards = text::guards_user(&checked.guards);
        let (failing, advisory): (Vec<_>, Vec<_>) =
            checked.flagged.into_iter().partition(Flagged::fails);
        let user = if failing.is_empty() {
            text::passed(blocks > 0, &advisory)
        } else if turn.blocked_tree.as_deref() == Some(now.as_str()) {
            Some(text::let_through(
                failing.len(),
                text::LetThrough::Unchanged,
            ))
        } else if blocks >= text::MAX_BLOCKS {
            Some(text::let_through(failing.len(), text::LetThrough::Cap))
        } else {
            let user = text::joined(Some(text::blocked(failing.len(), blocks + 1)), guards, " ");
            return self.block(turn, &failing, blocks + 1, now, user);
        };
        // The turn is over: the next one starts from here, which keeps a
        // setup without the turn-start hook checking one turn at a time.
        let _ = turn::save(&self.root, &Turn::begin(&self.event.session, now, None));
        Reply {
            user: text::joined(user, guards, " "),
            ..Reply::default()
        }
    }

    /// Block the stop with `failing`, recording the block, and tell the
    /// person `user`. A block that cannot be recorded is not made, so the
    /// cap always holds.
    fn block(
        &self,
        mut turn: Turn,
        failing: &[Flagged],
        block: u32,
        now: String,
        user: Option<String>,
    ) -> Reply {
        let reason = text::block_reason(failing, block);
        turn.blocks = block;
        turn.block_line = reason.lines().next().map(str::to_string);
        turn.blocked_tree = Some(now);
        if let Err(error) = turn::save(&self.root, &turn) {
            return failed(self.event, "this turn", &format!("{error:#}"));
        }
        Reply {
            block: true,
            agent: Some(reason),
            user,
        }
    }

    /// A stop that could not be checked: the person is told now, and the
    /// agent at the next event that carries context. The turn keeps its
    /// start, so the next check still covers these changes.
    fn unchecked(&self, mut turn: Turn, reason: &str) -> Reply {
        turn.notice = Some(text::failed_agent("the last turn's changes", reason));
        let _ = turn::save(&self.root, &turn);
        Reply {
            user: Some(text::failed_user("this turn", reason)),
            ..Reply::default()
        }
    }

    /// Run one check with the repository's configuration.
    fn check(&self, scope: review::Scope) -> Result<Checked, String> {
        let context = config::ConfigContext::discover_in(&self.cwd, None)
            .map_err(|error| format!("{error:#}"))?;
        review::check(
            context,
            scope,
            Arc::clone(&self.host.evaluators),
            self.deadline,
        )
    }
}

/// `files` relative to `root`, as the agent and the person read them: with
/// `/` on every platform, as the finding lines name them.
fn relative(root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    files
        .iter()
        .map(|file| crate::discovery::relative(file, root).unwrap_or_else(|_| file.clone()))
        .collect()
}
