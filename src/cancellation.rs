//! Cooperative cancellation; signal handlers only set an atomic flag.
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

static SIGNAL: OnceLock<Result<Arc<AtomicUsize>, String>> = OnceLock::new();

/// Install process-wide SIGINT/SIGTERM handling before any source writes.
pub fn install() -> io::Result<()> {
    match SIGNAL.get_or_init(register) {
        Ok(_) => check(),
        Err(message) => Err(io::Error::other(message.clone())),
    }
}

fn register() -> Result<Arc<AtomicUsize>, String> {
    let signal = Arc::new(AtomicUsize::new(0));
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    for number in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register_usize(number, Arc::clone(&signal), number as usize)
            .map_err(|error| format!("failed to install cancellation handler: {error}"))?;
    }
    Ok(signal)
}

/// The observed signal is retained through cleanup and final exit handling.
pub fn signal() -> Option<i32> {
    let number = SIGNAL.get()?.as_ref().ok()?.load(Ordering::Relaxed);
    (number != 0).then_some(number as i32)
}

/// Cancellation is an evaluation failure, never a killed mutant or a pass.
pub fn check() -> io::Result<()> {
    match signal() {
        Some(number) => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!("cancelled by signal {number}"),
        )),
        None => Ok(()),
    }
}
