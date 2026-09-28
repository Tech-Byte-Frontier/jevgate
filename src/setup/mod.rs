//! `jevgate init --agent`: a coding agent's hooks, and a short text telling
//! it how JevGate's findings work, in the agent's own files, for one user or
//! one repository. JevGate's parts are merged into what is there and
//! nothing else changes; run again, it writes nothing, and `--remove` takes
//! its parts out. Every file is planned before the first is written, so a
//! file JevGate cannot read whole leaves every file as it was.
mod agents;
mod hooks;
mod json;
#[cfg(test)]
mod packages;
mod probe;
#[cfg(test)]
mod tests;
mod text;

pub use agents::Target;

use agents::{Part, Places};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    fs,
    path::{Path, PathBuf},
};

/// A file larger than this is not an agent's settings or instructions.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// `jevgate init`'s arguments for coding agents.
#[derive(clap::Args, Debug, Default)]
pub struct AgentSetup {
    /// Set up a coding agent instead of writing jevgate.toml (repeatable, or comma-separated)
    ///
    /// Writes the agent's hooks, which run `jevgate hook`, and a short text
    /// telling the agent how JevGate's findings work, merged into the files
    /// already there. Your own settings (every repository) unless --project.
    #[arg(
        long = "agent",
        value_enum,
        value_name = "AGENT",
        value_delimiter = ','
    )]
    pub agents: Vec<Target>,
    /// With --agent: write the repository's agent files, for everyone who works in it
    ///
    /// They go at the top of the Git work tree: `.claude/`, `.codex/`,
    /// `.gemini/`, `.cursor/`, `.opencode/`, AGENTS.md and GEMINI.md.
    #[arg(long, requires = "agents")]
    pub project: bool,
    /// With --agent: take out the hooks and text JevGate wrote, and nothing else
    #[arg(long, requires = "agents")]
    pub remove: bool,
    /// With --agent: print what would change, and write nothing
    #[arg(long, requires = "agents")]
    pub dry_run: bool,
}

/// `jevgate init --agent`: plan every file, write them, and say what changed.
pub fn run(setup: &AgentSetup) -> Result<u8> {
    let cwd = std::env::current_dir()?.canonicalize()?;
    let places = Places::from_env(project_root(&cwd))?;
    let plan = Plan::new(setup, &places)?;
    if !setup.dry_run {
        plan.apply()?;
    }
    plan.report(setup, &places, std::env::var_os("PATH").as_deref());
    Ok(0)
}

/// The top of the Git work tree around `dir`, where agents read a
/// repository's settings, else `dir`.
fn project_root(dir: &Path) -> PathBuf {
    dir.ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(dir)
        .to_path_buf()
}

/// Whether `path`, followed through every symlink in it, stays inside
/// `root` (a resolved directory). A repository can make `AGENTS.md` or
/// `.codex` a link to any file of the person who runs `--project` in it,
/// such as `~/.bashrc`; its files must be its own. The part of the path that
/// does not exist yet is taken as written.
fn inside(path: &Path, root: &Path) -> bool {
    let mut existing = path;
    let mut missing = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            let whole = missing
                .iter()
                .rev()
                .fold(real, |path, name| path.join(name));
            return whole.starts_with(root);
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                existing = parent;
            }
            _ => return false,
        }
    }
}

/// Every change of one run, planned before the first write.
struct Plan {
    /// Each agent, and what happens to each of its files.
    agents: Vec<(Target, Vec<Step>)>,
    /// Each file's text on disk and its planned text; `None` is no file.
    files: BTreeMap<PathBuf, Planned>,
}

struct Planned {
    before: Option<String>,
    after: Option<String>,
}

/// What happens to one file of one agent.
struct Step {
    path: PathBuf,
    part: Part,
    outcome: Outcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Create,
    Update,
    Remove,
    Unchanged,
    Absent,
}

impl Outcome {
    fn of(before: Option<&str>, after: Option<&str>) -> Self {
        match (before, after) {
            (None, None) => Self::Absent,
            (None, Some(_)) => Self::Create,
            (Some(_), None) => Self::Remove,
            (Some(before), Some(after)) if before == after => Self::Unchanged,
            (Some(_), Some(_)) => Self::Update,
        }
    }

    fn verb(self, dry_run: bool) -> &'static str {
        match (self, dry_run) {
            (Self::Create, false) => "created",
            (Self::Create, true) => "would create",
            (Self::Update, false) => "updated",
            (Self::Update, true) => "would update",
            (Self::Remove, false) => "removed",
            (Self::Remove, true) => "would remove",
            (Self::Unchanged | Self::Absent, _) => "unchanged",
        }
    }
}

impl Plan {
    fn new(setup: &AgentSetup, places: &Places) -> Result<Self> {
        let mut plan = Self {
            agents: Vec::new(),
            files: BTreeMap::new(),
        };
        for (index, target) in setup.agents.iter().enumerate() {
            if setup.agents[..index].contains(target) {
                continue;
            }
            let steps = agents::files(*target, places, setup.project)
                .into_iter()
                .map(|(path, part)| {
                    let shown = places.show(&path);
                    if setup.project && !inside(&path, &places.root) {
                        bail!(
                            "Cannot set up {} in {shown}: a symlink leads it outside the repository, where JevGate does not write; nothing was written",
                            target.name()
                        );
                    }
                    plan.step(path, part, setup.remove).with_context(|| {
                        format!(
                            "Cannot set up {} in {shown}; nothing was written",
                            target.name()
                        )
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            plan.agents.push((*target, steps));
        }
        Ok(plan)
    }

    /// Put `part` into `path`, or with `remove` take it out, after what this
    /// run already planned for the file (Codex and OpenCode share AGENTS.md).
    fn step(&mut self, path: PathBuf, part: Part, remove: bool) -> Result<Step> {
        let planned = match self.files.entry(path.clone()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let text = read(&path)?;
                entry.insert(Planned {
                    before: text.clone(),
                    after: text,
                })
            }
        };
        let current = planned.after.clone();
        let next = change(current.as_deref(), &part, remove)?;
        let outcome = Outcome::of(current.as_deref(), next.as_deref());
        planned.after = next;
        Ok(Step {
            path,
            part,
            outcome,
        })
    }

    /// Write every changed file; a failure names the files already written.
    fn apply(&self) -> Result<()> {
        let mut written = Vec::new();
        for (path, planned) in &self.files {
            if planned.after == planned.before {
                continue;
            }
            let result = match &planned.after {
                Some(text) => write(path, text),
                None => delete(path),
            };
            if let Err(error) = result {
                let done = if written.is_empty() {
                    "nothing was written".to_string()
                } else {
                    format!("already written: {}", written.join(", "))
                };
                bail!("{error:#}; {done}");
            }
            written.push(path.display().to_string());
        }
        Ok(())
    }

    /// Say what changed in each agent's files, then what to check and do next.
    fn report(&self, setup: &AgentSetup, places: &Places, path: Option<&std::ffi::OsStr>) {
        let scope = if setup.project {
            format!("for the repository at {}", places.root.display())
        } else {
            "for your user (every repository)".to_string()
        };
        for (target, steps) in &self.agents {
            say!("{}, {scope}:", target.name());
            for step in steps {
                say!("  {}", step.line(setup, places));
            }
            if !setup.remove {
                for note in notes(*target) {
                    say!("  {note}");
                }
            }
        }
        if !setup.remove {
            self.advise(setup, places, path);
        }
    }

    /// After writing hooks: what would keep them from working as meant, and
    /// what the person does next.
    fn advise(&self, setup: &AgentSetup, places: &Places, path: Option<&std::ffi::OsStr>) {
        for warning in self
            .warnings(setup, places)
            .into_iter()
            .chain(probe::problem(path))
        {
            note!("jevgate: {warning}");
        }
        if setup.project && !places.root.join(crate::init::CONFIG_FILE).exists() {
            say!(
                "No {} in {}: checks use the defaults, and `jevgate init` writes one to review.",
                crate::init::CONFIG_FILE,
                places.root.display()
            );
        }
        if !setup.project {
            say!(
                "These hooks check every Git repository you run the agent in, and upload what `jevgate check` would there (a repository's {} bounds it); --project sets up one repository instead.",
                crate::init::CONFIG_FILE
            );
        }
        if !setup.dry_run {
            say!(
                "Next: `jevgate auth login` saves your TypeSafe key if you have not; the agent loads its hooks when a session starts."
            );
        }
    }

    /// What keeps the hooks from working as meant after this run: a
    /// repository outside Git, where the hook cannot tell what a turn changed,
    /// or JevGate running twice in one agent (the plugin beside Claude Code's
    /// hooks, or Cursor running both its own hooks and Claude Code's).
    fn warnings(&self, setup: &AgentSetup, places: &Places) -> Vec<String> {
        let setting_up = |target| self.agents.iter().any(|(t, _)| *t == target);
        let hooked = |target| {
            places
                .hook_files(target)
                .iter()
                .any(|path| self.settings(path).is_some_and(|s| hooks::handlers(&s) > 0))
        };
        let mut warnings = Vec::new();
        if setup.project && !places.root.join(".git").exists() {
            warnings.push(format!(
                "{} is not in a Git repository: the hook compares snapshots Git takes, so there it only says it could not check",
                places.root.display()
            ));
        }
        if setting_up(Target::Claude) && self.plugin_enabled(places) {
            warnings.push(
                "the JevGate plugin is enabled in Claude Code and runs the same hooks: keep one of them (`/plugin` disables the plugin; `jevgate init --agent claude --remove` takes these out)"
                    .to_string(),
            );
        }
        if (setting_up(Target::Claude) || setting_up(Target::Cursor))
            && hooked(Target::Claude)
            && hooked(Target::Cursor)
        {
            warnings.push(
                "Cursor also runs the hooks in Claude Code's settings (Settings > Agents > Third-Party Imports, on by default), so JevGate would run twice in Cursor: keep one of them, or turn that import off"
                    .to_string(),
            );
        }
        warnings
    }

    /// Whether a Claude Code settings file enables a plugin named `jevgate`.
    fn plugin_enabled(&self, places: &Places) -> bool {
        let files = [
            places.claude.join("settings.json"),
            places.root.join(".claude/settings.json"),
            places.root.join(".claude/settings.local.json"),
        ];
        files.iter().filter_map(|path| self.settings(path)).any(|settings| {
            matches!(settings.get("enabledPlugins"), Some(json::Json::Object(plugins))
                if plugins.iter().any(|(name, on)| name.starts_with("jevgate@") && *on == json::Json::Bool(true)))
        })
    }

    /// A settings file as it stands after this run, if it is readable JSON.
    fn settings(&self, path: &Path) -> Option<json::Json> {
        let text = match self.files.get(path) {
            Some(planned) => planned.after.clone()?,
            None => read(path).ok()??,
        };
        json::parse(&text).ok().map(|(settings, _)| settings)
    }
}

impl Step {
    fn line(&self, setup: &AgentSetup, places: &Places) -> String {
        let path = places.show(&self.path);
        let verb = self.outcome.verb(setup.dry_run);
        match (setup.remove, self.outcome) {
            (false, _) => format!("{verb} {path}: {}", self.part.describe()),
            (true, Outcome::Update) => format!("{verb} {path}: took out {}", self.part.noun()),
            (true, Outcome::Remove) => format!("{verb} {path}"),
            (true, Outcome::Absent) => format!("{path}: not there"),
            (true, _) => format!("{verb} {path}: nothing of JevGate's in it"),
        }
    }
}

/// What the person does after setting up `target`, beyond the hooks.
fn notes(target: Target) -> &'static [&'static str] {
    match target {
        Target::Codex => &[
            "Codex runs new or changed hooks only once you trust them: open /hooks in Codex and trust JevGate's.",
            "On macOS and Linux, Codex starts hooks from a login shell: jevgate must be on the PATH your login profile sets, not only in .zshrc or .bashrc.",
        ],
        Target::Gemini => &[
            "With Gemini CLI's security.environmentVariableRedaction on, hooks do not get TYPESAFE_API_KEY: save your key with `jevgate auth login`, or keep it in the repository's .env.",
        ],
        Target::Opencode => &[
            "OpenCode loads the plugin when it starts. OpenCode 2 runs a different plugin API and does not load it yet.",
        ],
        Target::Claude | Target::Cursor => &[],
    }
}

/// `current` (a file's text, `None` when there is no file) with JevGate's
/// `part` put in, or with `remove` taken out; `None` removes the file.
fn change(current: Option<&str>, part: &Part, remove: bool) -> Result<Option<String>> {
    match part {
        Part::Hooks(set) => {
            let (mut settings, layout) = json::parse(current.unwrap_or_default())?;
            if remove {
                if hooks::remove(&mut settings) == 0 {
                    return Ok(current.map(str::to_string));
                }
                if hooks::bare(&settings) {
                    return Ok(None);
                }
            } else {
                hooks::install(&mut settings, set)?;
            }
            Ok(Some(json::render(&settings, &layout)))
        }
        Part::Block if remove => current.map_or(Ok(None), text::without_block),
        Part::Block => text::with_block(current.unwrap_or_default(), text::INSTRUCTIONS).map(Some),
        Part::Owned { content, .. } => match current {
            Some(existing) if !text::owned(existing) && remove => Ok(Some(existing.to_string())),
            Some(existing) if !text::owned(existing) => {
                bail!(
                    "it was not written by JevGate, so it is left alone; move it away to set up this agent"
                )
            }
            _ => Ok((!remove).then(|| content.clone())),
        },
    }
}

/// A file's text, `None` when there is no file.
fn read(path: &Path) -> Result<Option<String>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("Cannot read {}", path.display())),
    };
    if metadata.len() > MAX_BYTES {
        bail!("{} is larger than {MAX_BYTES} bytes", path.display());
    }
    let bytes = fs::read(path).with_context(|| format!("Cannot read {}", path.display()))?;
    String::from_utf8(bytes)
        .map(Some)
        .with_context(|| format!("{} is not UTF-8 text", path.display()))
}

/// Write `text` through a temporary file and a rename, so an agent never
/// reads half a file. A symlink's target is written, not the link, and an
/// existing file keeps its permissions.
fn write(path: &Path, text: &str) -> Result<()> {
    let target = if path.is_symlink() {
        fs::canonicalize(path).with_context(|| format!("Cannot follow {}", path.display()))?
    } else {
        path.to_path_buf()
    };
    let (Some(directory), Some(name)) = (target.parent(), target.file_name()) else {
        bail!("Cannot write {}", target.display());
    };
    fs::create_dir_all(directory)
        .with_context(|| format!("Cannot create {}", directory.display()))?;
    let temporary = directory.join(format!(
        ".{}.jevgate-{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        fs::write(&temporary, text)?;
        if let Ok(metadata) = fs::metadata(&target) {
            fs::set_permissions(&temporary, metadata.permissions())?;
        }
        fs::rename(&temporary, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("Cannot write {}", target.display()))
}

/// Remove a file JevGate's parts alone filled. A symlink is kept and its
/// target emptied instead, since the link is often someone's dotfiles.
fn delete(path: &Path) -> Result<()> {
    if path.is_symlink() {
        let empty = if path.extension().is_some_and(|e| e == "json") {
            "{}\n"
        } else {
            ""
        };
        return write(path, empty);
    }
    fs::remove_file(path).with_context(|| format!("Cannot remove {}", path.display()))
}
