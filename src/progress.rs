//! A status line on stderr while a check runs, on a terminal only: what it
//! is doing, how many of the requests it has sent so far are answered, and
//! how long it has run. A whole-repository check of a 950-file project asks
//! Jev about 1,000 times, most of a minute at the 1,200 requests a minute
//! TypeSafe allows, and printed nothing until it ended.
//!
//! Every other line JevGate prints ([`note!`], [`say!`]) erases the status
//! line first, and the line is drawn again on the next tick.
use std::{
    io::{IsTerminal, Write},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// How often the line is drawn again.
const TICK: Duration = Duration::from_millis(125);
/// A check that ends sooner never shows the line, so a cached rerun does not flicker.
const QUIET: Duration = Duration::from_millis(400);

struct State {
    started: Instant,
    /// What the check is doing, such as `first pass`.
    phase: &'static str,
    /// Requests of this phase sent to the provider, and those answered.
    sent: usize,
    answered: usize,
    /// Whether the line is on the terminal now.
    shown: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Draws the line until dropped, then erases it.
pub struct Progress {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Whether a check's status line would be read as it is drawn: stderr is a
/// terminal that understands erasing a line, and no CI log records it.
pub fn wanted() -> bool {
    std::io::stderr().is_terminal()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
        && std::env::var_os("CI").is_none_or(|ci| ci.is_empty())
}

/// Start drawing the line when `enabled`.
pub fn start(enabled: bool) -> Option<Progress> {
    if !enabled {
        return None;
    }
    *lock() = Some(State {
        started: Instant::now(),
        phase: "reading files",
        sent: 0,
        answered: 0,
        shown: false,
    });
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        while !stopped.load(Ordering::Acquire) {
            std::thread::sleep(TICK);
            draw();
        }
    });
    Some(Progress {
        stop,
        thread: Some(thread),
    })
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let mut state = lock();
        erase(&mut state);
        *state = None;
    }
}

/// The check now does `phase`; its request counts start again.
pub fn phase(phase: &'static str) {
    if let Some(state) = lock().as_mut() {
        state.phase = phase;
        state.sent = 0;
        state.answered = 0;
    }
}

/// `n` more requests of this phase go to the provider.
pub fn sending(n: usize) {
    if let Some(state) = lock().as_mut() {
        state.sent += n;
    }
}

/// One request of this phase came back.
pub fn answered() {
    if let Some(state) = lock().as_mut() {
        state.answered += 1;
    }
}

/// Erases the line while held, so another line can be printed; the next
/// tick draws it again.
pub struct Paused {
    _held: MutexGuard<'static, Option<State>>,
}

/// Erase the line until the guard is dropped.
pub fn pause() -> Paused {
    let mut state = lock();
    erase(&mut state);
    Paused { _held: state }
}

fn lock() -> MutexGuard<'static, Option<State>> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn erase(state: &mut Option<State>) {
    if let Some(state) = state.as_mut().filter(|state| state.shown) {
        let _ = write!(std::io::stderr(), "\r\x1b[2K");
        state.shown = false;
    }
}

fn draw() {
    let mut state = lock();
    let Some(state) = state.as_mut() else {
        return;
    };
    let elapsed = state.started.elapsed();
    if elapsed < QUIET {
        return;
    }
    let line = line(state.phase, state.answered, state.sent, elapsed);
    let _ = write!(std::io::stderr(), "\r\x1b[2K{line}");
    let _ = std::io::stderr().flush();
    state.shown = true;
}

/// `JevGate · first pass · 312/1,126 answered · 23s`, short enough not to
/// wrap on an 80-column terminal, where erasing it would leave a line.
fn line(phase: &str, answered: usize, sent: usize, elapsed: Duration) -> String {
    let requests = if sent == 0 {
        String::new()
    } else {
        format!(" · {answered}/{sent} answered")
    };
    format!("JevGate · {phase}{requests} · {}", clock(elapsed.as_secs()))
}

fn clock(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        _ => format!("{}m {:02}s", seconds / 60, seconds % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_says_the_phase_the_requests_answered_and_the_time() {
        assert_eq!(
            line("first pass", 312, 1126, Duration::from_secs(23)),
            "JevGate · first pass · 312/1126 answered · 23s"
        );
        assert_eq!(
            line("planning", 0, 0, Duration::from_secs(83)),
            "JevGate · planning · 1m 23s"
        );
        assert!(
            line(
                "rechecking undecided units",
                9999,
                9999,
                Duration::from_secs(3599)
            )
            .chars()
            .count()
                < 80
        );
    }
}
