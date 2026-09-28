//! The `jevgate` a hook's command starts: the first one on `PATH`, asked to
//! answer an event it ignores. A missing one, a JevGate older than the hook
//! (it exits 2 on `hook`, which Claude Code reads as "erase the prompt") or
//! another program named `jevgate` is told to the person when the hooks are
//! written, since the agent would only show a failed hook later.
use std::{
    ffi::OsStr,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

/// An event every agent adapter ignores: the hook answers `{}` without
/// opening a repository.
const EVENT: &str = r#"{"hook_event_name":"JevGateSetupCheck"}"#;
/// A hook answers an ignored event at once; a program this slow is not one.
const WAIT: Duration = Duration::from_secs(10);
const INSTALL: &str = "https://tech-byte-frontier.github.io/jevgate/install.html";

/// What is wrong with the `jevgate` that `path` (a `PATH` value) leads the
/// agents to, if anything.
pub(super) fn problem(path: Option<&OsStr>) -> Option<String> {
    let Some(found) = path.and_then(|path| find(path, "jevgate")) else {
        return Some(format!(
            "no jevgate is on your PATH, and the hooks run `jevgate hook` by name: install JevGate where the agent finds it ({INSTALL}; with npm, `npm install -g @tech-byte-frontier/jevgate`)"
        ));
    };
    answer(&found).err().map(|why| {
        format!(
            "the jevgate on your PATH ({}) cannot answer the hooks: {why}. The agent runs that one: replace it with JevGate 0.27 or later, or put one first on your PATH (the npm package named jevgate is another program; JevGate's is @tech-byte-frontier/jevgate)",
            found.display()
        )
    })
}

/// The first program `name` in `path`, as a shell would find it: with
/// Windows' executable extensions, or with an executable bit elsewhere.
/// Directories npm adds only while `npx` or a package script runs are
/// passed over: `npx @tech-byte-frontier/jevgate init --agent claude` runs
/// with npx's own copy first on `PATH`, which is gone when the agent starts
/// its hooks.
pub(super) fn find(path: &OsStr, name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        extensions
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(|extension| format!("{name}{}", extension.to_ascii_lowercase()))
            .collect()
    } else {
        vec![name.to_string()]
    };
    std::env::split_paths(path)
        .filter(|directory| !npm_run_only(directory))
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| executable(candidate))
}

/// Whether npm puts `directory` on `PATH` only for the command it runs:
/// npx's cache (`~/.npm/_npx/HASH/node_modules/.bin`) and a package's
/// `node_modules/.bin`.
fn npm_run_only(directory: &Path) -> bool {
    directory.ends_with("node_modules/.bin")
        || directory.components().any(|c| c.as_os_str() == "_npx")
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
    path.is_file()
}

/// Run `program hook` on an event it ignores: it must exit 0 and print `{}`.
pub(super) fn answer(program: &Path) -> Result<(), String> {
    let mut child = Command::new(program)
        .arg("hook")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("it did not start ({error})"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A program that exits without reading stdin is judged by its answer.
        let _ = stdin.write_all(EVENT.as_bytes());
    }
    let output = finish(child)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() && stdout.trim() == "{}" {
        return Ok(());
    }
    Err(match output.status.code() {
        Some(0) => format!("it answered {:?}, not {{}}", first_line(&stdout)),
        code => format!(
            "`jevgate hook` exited {} ({})",
            code.map_or_else(|| "on a signal".to_string(), |code| code.to_string()),
            first_line(&String::from_utf8_lossy(&output.stderr))
        ),
    })
}

/// What `child` printed once it exits, or why it did not within [`WAIT`].
fn finish(child: Child) -> Result<Output, String> {
    match crate::child::output_until(child, Instant::now() + WAIT) {
        Ok(Some(output)) => Ok(output),
        Ok(None) => Err(format!("it did not answer within {} s", WAIT.as_secs())),
        Err(error) => Err(format!("it could not be waited for ({error})")),
    }
}

/// The first line of a program's output, cut for a sentence.
fn first_line(text: &str) -> String {
    /// Enough of a line to recognize an error or a usage line.
    const SHOWN: usize = 160;
    text.trim()
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(SHOWN)
        .collect()
}
