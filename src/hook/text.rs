//! What the hook says: one line per finding for the agent, within what every
//! agent reads whole, and short notes for the person. Context states facts;
//! only the reason of a block, which the agent is meant to act on, instructs.
use super::review::Flagged;
use crate::{
    guards::{self, Guard, Kind},
    output,
    schema::Strength,
};
use std::path::PathBuf;

/// Findings listed in one reply; the rest are counted.
const SHOWN: usize = 10;
/// A reply stays under this many characters: Claude Code reads hook text
/// whole up to 10,000, Copilot 10 KB, Codex about 2,500 tokens.
const MAX_CHARS: usize = 8_000;
/// Room kept for the line counting the findings left out.
const REST_CHARS: usize = 160;
/// A finding's why and next step are cut at these lengths.
const WHY_CHARS: usize = 300;
const NEXT_CHARS: usize = 200;
/// An error quoted in a notice is cut at this length.
const REASON_CHARS: usize = 400;
/// Blocks per turn before the hook lets the agent finish.
pub(super) const MAX_BLOCKS: u32 = 3;
/// Where the person sees findings the hook did not send the agent.
const LIST_THEM: &str = "`jevgate check --base HEAD` lists them.";
/// Guards listed after an edit, and named in the person's note, at most.
const SHOWN_GUARDS: usize = 5;
const NAMED_GUARDS: usize = 3;
/// A guard's line is cut at this length.
const GUARD_CHARS: usize = 240;

/// The context after an edit: the findings the agent was not given yet this
/// turn, a count of the `known` ones, and the `guards` it was not told of
/// yet; nothing when there is none of them. Guards take their room first.
pub(super) fn after_edit(
    files: &[PathBuf],
    new: &[Flagged],
    known: &[Flagged],
    guards: &[&Guard],
) -> Option<String> {
    let noticed = guards_noticed(guards);
    let room = MAX_CHARS.saturating_sub(noticed.as_ref().map_or(0, |n| n.len() + 2));
    joined(
        findings_after_edit(files, (new, known), room),
        noticed,
        "\n\n",
    )
}

/// The findings part of the context after an edit, within `room` characters.
fn findings_after_edit(
    files: &[PathBuf],
    (new, known): (&[Flagged], &[Flagged]),
    room: usize,
) -> Option<String> {
    let reviewed = format!("JevGate reviewed {} after this edit:", named(files));
    let earlier = format!(
        "{} reported earlier this turn {} ({} the quality gate).",
        output::count(known.len(), "finding"),
        if known.len() == 1 {
            "remains"
        } else {
            "remain"
        },
        fail(known.iter().filter(|f| f.fails).count())
    );
    match (new.is_empty(), known.is_empty()) {
        (true, true) => return None,
        (true, false) => return Some(format!("{reviewed} {earlier}")),
        _ => {}
    }
    let failing = new.iter().filter(|f| f.fails).count();
    let head = format!(
        "{reviewed} {}, {} the quality gate.",
        output::count(
            new.len(),
            if known.is_empty() {
                "finding"
            } else {
                "new finding"
            }
        ),
        fail(failing)
    );
    let blocks = if failing > 0 || known.iter().any(|f| f.fails) {
        "Findings that fail the gate block the end of the turn until they are fixed; the others are optional."
    } else {
        "None of them blocks the end of the turn."
    };
    let tail = if known.is_empty() {
        blocks.to_string()
    } else {
        format!("{earlier}\n{blocks}")
    };
    Some(list(&head, new, &tail, room))
}

/// The reason a stop is blocked, which the agent reads as its next
/// instruction. Its first line names the block, so a prompt that repeats it
/// is known as this turn's continuation.
pub(super) fn block_reason(failing: &[Flagged], block: u32) -> String {
    let head = format!(
        "JevGate blocked the end of this turn ({block} of at most {MAX_BLOCKS}): {} in code changed this turn {} the quality gate.",
        output::count(failing.len(), "finding"),
        if failing.len() == 1 { "fails" } else { "fail" }
    );
    let mut tail = String::from(
        "Fix them, then finish. If a finding is mistaken, keep the code as it is and say why in your reply; JevGate does not block again when nothing changed.",
    );
    if failing.iter().any(|f| f.accepted_this_turn) {
        tail.push_str(" A finding accepted this turn, by a baseline entry or a `jevgate: allow` comment, counts until the next turn: accepting findings is the person's call, so leave that to them.");
    }
    list(&head, failing, &tail, MAX_CHARS)
}

/// The context after an edit about what the turn did to the checks around
/// the code, and who is told; nothing when it did nothing.
fn guards_noticed(guards: &[&Guard]) -> Option<String> {
    if guards.is_empty() {
        return None;
    }
    let mut text = format!(
        "JevGate noticed that this turn {} so far:\n",
        guards::summary(guards.iter().copied())
    );
    for guard in guards.iter().take(SHOWN_GUARDS) {
        text.push_str(&format!("- {}\n", clip(&guard.describe(), GUARD_CHARS)));
    }
    if guards.len() > SHOWN_GUARDS {
        text.push_str(&format!(
            "{} not shown.\n",
            output::count(guards.len() - SHOWN_GUARDS, "more")
        ));
    }
    text.push_str("JevGate reports these to the person at the end of the turn. Within a turn it reads jevgate.toml, the baseline and `jevgate: allow` comments as they were when the turn began.");
    Some(text)
}

/// The person's note on what a turn did to the checks around the code, and
/// what the gate read when the turn edited them.
pub(super) fn guards_user(guards: &[Guard]) -> Option<String> {
    if guards.is_empty() {
        return None;
    }
    let mut named: Vec<String> = guards
        .iter()
        .take(NAMED_GUARDS)
        .map(|g| clip(&g.describe(), GUARD_CHARS))
        .collect();
    if guards.len() > NAMED_GUARDS {
        named.push(format!("{} more", guards.len() - NAMED_GUARDS));
    }
    let mut text = format!(
        "JevGate: this turn {} ({}).",
        guards::summary(guards),
        named.join("; ")
    );
    let edited = |g: &Guard| matches!(g.kind, Kind::Allow | Kind::Configuration | Kind::Baseline);
    if guards.iter().any(edited) {
        text.push_str(" Its gate read jevgate.toml, the baseline and `jevgate: allow` comments as they were when the turn began.");
    }
    Some(text)
}

/// Two optional texts, `between` them when both are there.
pub(super) fn joined(
    first: Option<String>,
    second: Option<String>,
    between: &str,
) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}{between}{second}")),
        (first, second) => first.or(second),
    }
}

/// The person's note on a block.
pub(super) fn blocked(failing: usize, block: u32) -> String {
    format!(
        "JevGate: {} in this turn's changes {} the quality gate; the agent is asked to fix {} (block {block} of {MAX_BLOCKS}).",
        output::count(failing, "finding"),
        if failing == 1 { "fails" } else { "fail" },
        if failing == 1 { "it" } else { "them" }
    )
}

/// Why a stop with findings that fail the gate lets the agent finish.
pub(super) enum LetThrough {
    /// The turn was blocked as often as the hook blocks one.
    Cap,
    /// Nothing changed since the last block.
    Unchanged,
}

pub(super) fn let_through(failing: usize, why: LetThrough) -> String {
    let still = format!(
        "{} still {} the quality gate",
        output::count(failing, "finding"),
        if failing == 1 { "fails" } else { "fail" }
    );
    match why {
        LetThrough::Cap => format!(
            "JevGate blocked this turn {MAX_BLOCKS} times and lets the agent finish; {still}. {LIST_THEM}"
        ),
        LetThrough::Unchanged => format!(
            "JevGate lets the agent finish: nothing changed after its last block, and {still}. {LIST_THEM}"
        ),
    }
}

/// The person's note on a stop that passes: whether the findings of a block
/// are fixed, and the findings that do not fail the gate.
pub(super) fn passed(after_block: bool, advisory: &[Flagged]) -> Option<String> {
    let mut notes = Vec::new();
    if after_block {
        notes.push("JevGate: the findings that blocked this turn are fixed.".to_string());
    }
    if !advisory.is_empty() {
        let reviews = advisory
            .iter()
            .filter(|f| f.finding.strength == Strength::Review)
            .count();
        let counts: Vec<String> = [(reviews, "review"), (advisory.len() - reviews, "consider")]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, noun)| output::count(*n, noun))
            .collect();
        notes.push(format!(
            "JevGate: {} in this turn's changes {} the quality gate. {LIST_THEM}",
            counts.join(" and "),
            if advisory.len() == 1 {
                "doesn't fail"
            } else {
                "don't fail"
            }
        ));
    }
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// The person's note on a stop with no record of the turn's start.
pub(super) const UNCHECKED_TURN: &str = "JevGate did not check this turn: it has no snapshot of the turn's start, so its prompt hook may not be installed. It checks from the next turn on.";

/// What the person is told when `what` could not be checked.
pub(super) fn failed_user(what: &str, why: &str) -> String {
    format!(
        "JevGate could not check {what}: {}. Nothing was blocked.",
        reason(why)
    )
}

/// What the agent is told when `what` could not be checked.
pub(super) fn failed_agent(what: &str, why: &str) -> String {
    format!(
        "JevGate could not check {what} ({}), so it was not reviewed; this is not a pass.",
        reason(why)
    )
}

/// The files an edit wrote, for a sentence: one or two by name, else a count.
pub(super) fn named(files: &[PathBuf]) -> String {
    match files {
        [one] => one.display().to_string(),
        [first, second] => format!("{} and {}", first.display(), second.display()),
        _ => output::count(files.len(), "file"),
    }
}

/// "none fails", "1 fails", "2 fail".
fn fail(n: usize) -> String {
    match n {
        0 => "none fails".into(),
        1 => "1 fails".into(),
        _ => format!("{n} fail"),
    }
}

/// `head`, a line per finding while the text stays under `room` characters,
/// how many were left out and where they are, then `tail`.
fn list(head: &str, found: &[Flagged], tail: &str, room: usize) -> String {
    let mut text = format!("{head}\n");
    let mut shown = 0;
    for flagged in found.iter().take(SHOWN) {
        let line = line(flagged);
        if text.len() + line.len() + REST_CHARS + tail.len() >= room {
            break;
        }
        text.push_str(&line);
        text.push('\n');
        shown += 1;
    }
    let rest = found.len() - shown;
    if rest > 0 {
        text.push_str(&format!(
            "{} not shown; .jevgate/latest.json holds every finding (the jevgate_findings tool reads it).\n",
            output::count(rest, "more finding")
        ));
    }
    text.push_str(tail);
    text
}

/// `- path:line level rule (fails the gate): why Next: step`.
fn line(flagged: &Flagged) -> String {
    let finding = &flagged.finding;
    let mark = match (flagged.fails, flagged.accepted_this_turn) {
        (true, true) => " (fails the gate; accepted this turn)",
        (true, false) => " (fails the gate)",
        (false, true) => " (accepted this turn)",
        (false, false) => "",
    };
    format!(
        "- {}:{} {} {}{mark}: {} Next: {}",
        flagged.path.display(),
        finding.line,
        output::label(&finding.strength),
        finding.rule,
        sentence(&finding.message, WHY_CHARS),
        sentence(&finding.action, NEXT_CHARS)
    )
}

/// `text` on one line, cut to `max` characters with an ellipsis.
fn clip(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", flat[..cut].trim_end()),
        None => flat,
    }
}

/// `text` clipped as one sentence, so the next one can follow it.
fn sentence(text: &str, max: usize) -> String {
    let clipped = clip(text, max);
    if clipped.ends_with(['.', '!', '?', '…']) || clipped.is_empty() {
        clipped
    } else {
        format!("{clipped}.")
    }
}

/// An error quoted inside a sentence: one line, no closing period.
fn reason(text: &str) -> String {
    clip(text, REASON_CHARS).trim_end_matches('.').to_string()
}
