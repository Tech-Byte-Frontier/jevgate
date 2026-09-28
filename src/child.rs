//! A child process waited for until a deadline. Its output is read as it
//! comes, so a long listing cannot fill a pipe and stall it, and it is
//! stopped at the deadline, so a caller with a time budget keeps it.
use std::{
    io::{self, Read},
    process::{Child, Output},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// The first pause between two looks at a running child; each later one
/// doubles, up to [`LONGEST_PAUSE`]. Git answers most calls within a few
/// milliseconds, which a fixed pause would round up.
const FIRST_PAUSE: Duration = Duration::from_millis(1);
const LONGEST_PAUSE: Duration = Duration::from_millis(16);

/// `child`'s exit status and output once it exits; none when it was still
/// running at `deadline`, when it is killed. Its stdout and stderr must be
/// piped, and its stdin written or closed by the caller.
pub(crate) fn output_until(mut child: Child, deadline: Instant) -> io::Result<Option<Output>> {
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let mut pause = FIRST_PAUSE;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(Output {
                status,
                stdout: collected(stdout),
                stderr: collected(stderr),
            }));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            // The readers are left behind: a process the child started, such
            // as a Git filter, may hold its pipes open after it is killed.
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        thread::sleep(pause.min(left));
        pause = (pause * 2).min(LONGEST_PAUSE);
    }
}

/// A thread reading `pipe` to its end.
fn drain(pipe: Option<impl Read + Send + 'static>) -> Option<JoinHandle<Vec<u8>>> {
    pipe.map(|mut pipe| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    })
}

fn collected(reader: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn started(script: &str) -> Child {
        Command::new("sh")
            .args(["-c", script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    #[test]
    fn a_child_that_prints_more_than_a_pipe_holds_is_read_to_its_end() {
        let child = started("head -c 300000 /dev/zero; echo done >&2");
        let output = output_until(child, Instant::now() + Duration::from_secs(20))
            .unwrap()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 300_000);
        assert_eq!(output.stderr, b"done\n");
    }

    #[test]
    fn a_child_still_running_at_the_deadline_is_stopped() {
        let begun = Instant::now();
        let child = started("sleep 5");
        let output = output_until(child, begun + Duration::from_millis(200)).unwrap();
        assert!(output.is_none());
        assert!(begun.elapsed() < Duration::from_secs(4), "it was stopped");
    }
}
