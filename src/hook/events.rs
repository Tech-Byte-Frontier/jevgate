//! What each event does in its repository: a session's or turn's start
//! records a snapshot of the working tree, an edit is checked and its
//! findings go to the agent, and a stop is checked against the turn's start
//! and blocked while findings fail the gate.
use super::{
    Host,
    agents::{self, Event, Kind, Reply},
    outage,
    review::{self, Checked, Flagged, Unreviewed},
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

/// The session's directory is not in a Git work tree, or Git cannot run.
#[derive(Debug)]
pub(super) struct OutsideGit(PathBuf);

impl std::error::Error for OutsideGit {}
impl std::fmt::Display for OutsideGit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is not in a Git repository (or Git cannot run), so JevGate cannot tell what a turn changed; it says so once a session",
            self.0.display()
        )
    }
}

/// Whether the session of `event` has not yet been told that its directory
/// is outside Git. Hooks set up for a user run in every directory an agent
/// opens, so a session there would otherwise hear it at every prompt, edit
/// and stop. An empty mark in the system's temporary directory remembers
/// it; when it cannot be written, the session is told again.
pub(super) fn first_outside(event: &Event) -> bool {
    let Some(directory) = marks_directory(&std::env::temp_dir()) else {
        return true;
    };
    let key = format!(
        "{}\n{}",
        event.session,
        event.cwd.as_deref().unwrap_or(Path::new("")).display()
    );
    let mark = directory.join(&crate::schema::hash(key.as_bytes())[..32]);
    turn::prune(&directory);
    !matches!(
        std::fs::OpenOptions::new().write(true).create_new(true).open(mark),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists
    )
}

/// The user's directory of outside-Git marks in `temporary`, created only
/// the user can open. None when that path is a symbolic link or not a
/// directory: in a shared /tmp another user could plant a link there, and
/// the week-old files `turn::prune` removes would be in the link's target.
pub(super) fn marks_directory(temporary: &Path) -> Option<PathBuf> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let user: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let directory = temporary.join(format!("jevgate-hook-{user}"));
    match std::fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata.is_dir().then_some(directory),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder.create(&directory).ok().map(|()| directory)
        }
        Err(_) => None,
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
    /// so no state is written into a directory that is not one. Outside one
    /// the error is [`OutsideGit`].
    pub(super) fn open(event: &'a Event, host: &'a Host, deadline: Instant) -> Result<Self> {
        let cwd = event.cwd.as_deref().unwrap_or(&host.cwd);
        let cwd = cwd
            .canonicalize()
            .with_context(|| format!("Cannot open the session's directory {}", cwd.display()))?;
        let root = config::repository_root(&cwd);
        let index = crate::revision::index_file(&root).ok_or_else(|| OutsideGit(root.clone()))?;
        Ok(Self {
            event,
            host,
            deadline,
            cwd,
            root,
            index,
        })
    }

    /// The reply to the event, `input` as the agent sent it: none when
    /// another `jevgate hook` process took the same event.
    pub(super) fn handle(&self, input: &serde_json::Value) -> Reply {
        let mut same = input.clone();
        if let Some(fields) = same.as_object_mut() {
            // Two hook files may name the event their own way.
            fields.remove("hook_event_name");
        }
        let key = format!("{:?} {:?} {same}", self.event.agent, self.event.kind);
        let Some(_answering) = turn::claim(&self.root, &key) else {
            return Reply::default();
        };
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
    /// one in the middle of a turn. The agent is told the hooks run.
    fn session_start(&self) -> Reply {
        let reply = match self.load() {
            Some(turn) => self.context(turn, None),
            None => self.begin(None),
        };
        greeted(reply)
    }

    /// A new turn begins, unless the prompt is this turn's block reason sent
    /// back (Gemini CLI and Cursor continue a blocked turn that way). After
    /// a stop that could not be checked, it begins where that turn did, so
    /// the changes are checked once JevGate can check them. A session's
    /// first turn tells the agent the hooks run, when its session start did
    /// not (OpenCode's plugin sends none).
    fn turn_start(&self) -> Reply {
        let previous = self.load();
        if previous.is_none() {
            return greeted(self.begin(None));
        }
        match previous {
            Some(turn) if turn.continued_by(&self.event.prompt) => self.context(turn, None),
            Some(turn) if turn.unchecked => {
                let turn = Turn::carry(&self.event.session, turn);
                match turn::save(&self.root, &turn) {
                    Ok(()) => self.context(turn, None),
                    Err(error) => failed(self.event, "this turn", &format!("{error:#}")),
                }
            }
            _ => {
                turn::prune(&self.root.join(".jevgate/turns"));
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

    /// Whether `file` is inside a Git repository of its own below the root,
    /// a submodule or a nested clone: a snapshot records only its commit.
    fn nested(&self, file: &Path) -> bool {
        file.ancestors()
            .skip(1)
            .take_while(|dir| *dir != self.root && dir.starts_with(&self.root))
            .any(|dir| dir.join(".git").exists())
    }

    /// Check the edited files since the turn began (whole, without a turn)
    /// and tell the agent their findings. Nothing blocks. Files inside a
    /// repository of their own are named as not reviewed, and remembered for
    /// the person at the end of the turn.
    fn after_edit(&self) -> Reply {
        let mut turn = self.load();
        let (files, unseen) = self.seen(turn.as_mut());
        if files.is_empty() {
            let checked = Checked {
                unreviewed: unseen,
                ..Checked::default()
            };
            return self.told(turn, &[], checked);
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
        let mut checked = match self.check(review::Scope {
            trees,
            paths: files,
        }) {
            Ok(checked) => checked,
            Err(unfinished) => return failed(self.event, &named, &unfinished.reason),
        };
        checked.unreviewed.extend(unseen);
        self.told(turn, &shown, checked)
    }

    /// The edited files the turn's snapshots see, and as not reviewed those
    /// inside a repository of their own, which `turn` remembers for the
    /// person.
    fn seen(&self, turn: Option<&mut Turn>) -> (Vec<PathBuf>, Vec<Unreviewed>) {
        let (nested, files): (Vec<PathBuf>, Vec<PathBuf>) = self
            .edited()
            .into_iter()
            .partition(|file| self.nested(file));
        let nested = relative(&self.root, &nested);
        if let Some(turn) = turn.filter(|_| !nested.is_empty()) {
            let paths = nested.iter().map(|path| path.display().to_string());
            if turn.remember_unseen(paths) {
                let _ = turn::save(&self.root, turn);
            }
        }
        (files, nested.into_iter().map(Unreviewed::unseen).collect())
    }

    /// The context telling the agent what `checked` found in the files
    /// `shown`, less what `turn` already told it, which it remembers.
    fn told(&self, turn: Option<Turn>, shown: &[PathBuf], checked: Checked) -> Reply {
        let Some(mut turn) = turn else {
            let undecided: Vec<_> = checked.undecided.iter().collect();
            let guards: Vec<_> = checked.guards.iter().collect();
            let unreviewed: Vec<_> = checked.unreviewed.iter().collect();
            return Reply {
                agent: text::after_edit(
                    shown,
                    (&checked.flagged, &[]),
                    &undecided,
                    &guards,
                    &unreviewed,
                ),
                ..Reply::default()
            };
        };
        let (known, new): (Vec<_>, Vec<_>) = checked
            .flagged
            .into_iter()
            .partition(|f| turn.reported.contains(&f.finding.fingerprint));
        let undecided: Vec<_> = checked
            .undecided
            .iter()
            .filter(|u| !turn.reported.contains(&u.id()))
            .collect();
        let guards: Vec<_> = checked
            .guards
            .iter()
            .filter(|g| !turn.reported.contains(&g.id))
            .collect();
        let unreviewed: Vec<_> = checked
            .unreviewed
            .iter()
            .filter(|u| !turn.reported.contains(&u.id()))
            .collect();
        let context = text::after_edit(shown, (&new, &known), &undecided, &guards, &unreviewed);
        let ids: Vec<String> = new
            .iter()
            .map(|f| f.finding.fingerprint.clone())
            .chain(undecided.iter().map(|u| u.id()))
            .chain(guards.iter().map(|g| g.id.clone()))
            .chain(unreviewed.iter().map(|u| u.id()))
            .collect();
        if turn.report(ids.iter().map(String::as_str)) {
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
            Err(unfinished) if unfinished.start_unreadable => {
                self.unreadable_start(now, &unfinished.reason)
            }
            Err(unfinished) => self.unchecked(turn, &unfinished.reason),
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
    fn turn_findings(&self, turn: &Turn, now: &str) -> Result<Checked, review::Unfinished> {
        if now == turn.tree {
            return Ok(Checked::default());
        }
        self.check(review::Scope {
            trees: Some((turn.tree.clone(), now.to_string())),
            paths: Vec::new(),
        })
    }

    /// Block while findings fail the gate, or units left undecided where
    /// undecided results fail it: at most three times a turn, and not again
    /// when nothing changed since the last block. A stop that is not blocked
    /// ends the turn. The person hears of the turn's guards.
    fn decide(&self, turn: Turn, now: String, mut checked: Checked) -> Reply {
        let blocks = if self.event.continued { turn.blocks } else { 0 };
        let unseen = turn
            .unseen
            .iter()
            .map(PathBuf::from)
            .map(Unreviewed::unseen);
        checked.unreviewed.extend(unseen);
        let guards = text::joined(
            text::unreviewed_user(&checked.unreviewed),
            text::guards_user(&checked.guards),
            " ",
        );
        let (failing, advisory): (Vec<_>, Vec<_>) =
            checked.flagged.into_iter().partition(Flagged::fails);
        let undecided = checked.undecided;
        let (failed, unsure) = (failing.len(), undecided.len());
        let user = if failed + unsure == 0 {
            text::passed(blocks > 0, &advisory)
        } else if turn.blocked_tree.as_deref() == Some(now.as_str()) {
            Some(text::let_through(
                failed,
                unsure,
                text::LetThrough::Unchanged,
            ))
        } else if blocks >= text::MAX_BLOCKS {
            Some(text::let_through(failed, unsure, text::LetThrough::Cap))
        } else {
            let user = text::joined(Some(text::blocked(failed, unsure, blocks + 1)), guards, " ");
            let blocking = Blocking {
                failing: &failing,
                undecided: &undecided,
                block: blocks + 1,
            };
            return self.block(turn, blocking, now, user);
        };
        // The turn is over: the next one starts from here, which keeps a
        // setup without the turn-start hook checking one turn at a time.
        let _ = turn::save(&self.root, &Turn::begin(&self.event.session, now, None));
        Reply {
            user: text::joined(user, guards, " "),
            ..Reply::default()
        }
    }

    /// Block the stop on what `blocking` names, recording the block, and
    /// tell the person `user`. A block that cannot be recorded is not made,
    /// so the cap always holds.
    fn block(
        &self,
        mut turn: Turn,
        blocking: Blocking,
        now: String,
        user: Option<String>,
    ) -> Reply {
        let Blocking {
            failing,
            undecided,
            block,
        } = blocking;
        let reason = text::block_reason(failing, undecided, block, turn.carried);
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

    /// A stop whose turn began with a configuration that does not load, such
    /// as a jevgate.toml the person broke: every check from that start fails
    /// the same way, so the next turn begins now, where the configuration
    /// may load again, and this turn's changes stay unchecked. The person is
    /// told now, and the agent at its next event.
    fn unreadable_start(&self, now: String, reason: &str) -> Reply {
        let notice = Some(text::unreadable_start_agent(reason));
        let _ = turn::save(&self.root, &Turn::begin(&self.event.session, now, notice));
        Reply {
            user: Some(text::unreadable_start_user(reason)),
            ..Reply::default()
        }
    }

    /// A stop that could not be checked: the person is told now, and the
    /// agent at the next event that carries context. The turn keeps its
    /// start, and the next turn begins there too, so the next check still
    /// covers these changes.
    fn unchecked(&self, mut turn: Turn, reason: &str) -> Reply {
        turn.notice = Some(text::unchecked_turn(reason));
        turn.unchecked = true;
        let _ = turn::save(&self.root, &turn);
        Reply {
            user: Some(text::failed_user("this turn", reason)),
            ..Reply::default()
        }
    }

    /// Run one check in the repository: without asking the provider while
    /// the hook waits out its failure, and waiting one out when the check
    /// meets it.
    fn check(&self, scope: review::Scope) -> Result<Checked, review::Unfinished> {
        let place = review::Place {
            cwd: self.cwd.clone(),
            root: self.root.clone(),
        };
        let waiting = outage::current(&self.root);
        let asking = review::Asking {
            evaluators: Arc::clone(&self.host.evaluators),
            deadline: self.deadline,
            waiting: waiting.as_ref().map(text::waiting),
        };
        match review::check(place, scope, asking) {
            Ok(checked) => {
                if waiting.is_none() {
                    outage::clear(&self.root);
                }
                Ok(checked)
            }
            Err(unfinished) => {
                if let Some(failure) = unfinished.outage.as_ref().filter(|_| waiting.is_none()) {
                    outage::record(&self.root, failure);
                }
                Err(unfinished)
            }
        }
    }
}

/// What a stop is blocked on, and which block of the turn it is.
struct Blocking<'a> {
    failing: &'a [Flagged],
    undecided: &'a [review::Undecided],
    block: u32,
}

/// `reply` opened with the line that tells the agent JevGate's hooks run in
/// its session. Its instructions ask it to check by hand without that line.
fn greeted(reply: Reply) -> Reply {
    Reply {
        agent: text::joined(Some(text::RUNNING.into()), reply.agent, "\n\n"),
        ..reply
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
