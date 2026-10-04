//! What the hook says: one line per finding for the agent, within what every
//! agent reads whole, and short notes for the person. Context states facts;
//! only the reason of a block, which the agent is meant to act on, instructs.
use super::review::{Dismissed, Flagged, Undecided, Unreviewed};
use crate::{
    guards::{self, Guard, Kind},
    output,
    view::FindingView,
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
/// How the agent dismisses a finding it judged, and who audits that.
const DISMISS: &str = "dismiss it with `jevgate baseline mark wrong|intended|later PATH:LINE`, which the person audits with `jevgate baseline stats`";
/// Dismissed findings named in the person's note, at most.
const NAMED_DISMISSED: usize = 3;
/// Guards and unreviewed files listed after an edit, and named in the
/// person's note, at most.
const SHOWN_GUARDS: usize = 5;
const NAMED_GUARDS: usize = 3;
/// A guard's line is cut at this length.
const GUARD_CHARS: usize = 240;

/// The context after an edit: the findings the agent was not given yet this
/// turn, a count of the `known` ones, then the undecided units that fail the
/// gate, the edited files the check did not judge and the `guards` it was
/// not told of yet; nothing when there is none of them. The notes take their
/// room first.
pub(super) fn after_edit(
    files: &[PathBuf],
    (new, known): (&[Flagged], &[Flagged]),
    undecided: &[&Undecided],
    guards: &[&Guard],
    unreviewed: &[&Unreviewed],
) -> Option<String> {
    let noticed = joined(
        undecided_agent(undecided),
        joined(unreviewed_agent(unreviewed), guards_noticed(guards), "\n\n"),
        "\n\n",
    );
    let room = MAX_CHARS.saturating_sub(noticed.as_ref().map_or(0, |n| n.len() + 2));
    joined(
        findings_after_edit(files, (new, known), room),
        noticed,
        "\n\n",
    )
}

/// The agent's note on units left undecided where undecided results fail
/// the gate: they block the end of the turn as failing findings do.
fn undecided_agent(units: &[&Undecided]) -> Option<String> {
    if units.is_empty() {
        return None;
    }
    let mut text = format!(
        "{} {}:",
        output::count(units.len(), "undecided unit"),
        if units.len() == 1 {
            "fails the quality gate, which fails on undecided results here"
        } else {
            "fail the quality gate, which fails on undecided results here"
        }
    );
    for unit in units.iter().take(SHOWN_GUARDS) {
        text.push_str(&format!("\n{}", undecided_line(unit)));
    }
    if units.len() > SHOWN_GUARDS {
        text.push_str(&format!(
            "\n{} not shown.",
            output::count(units.len() - SHOWN_GUARDS, "more")
        ));
    }
    text.push_str(&format!("\n{UNDECIDED_NEXT}"));
    Some(text)
}

/// What the agent can do about a unit Jev could not decide.
const UNDECIDED_NEXT: &str = "Jev could not decide these, and `uncertain` is among the levels jevgate.toml sets for them, so they block the end of the turn: make the code there clear enough to decide, or say why it is right as it is.";

/// `- path:line undecided rule (fails the gate): unit: questions.`
fn undecided_line(unit: &Undecided) -> String {
    format!(
        "- {}:{} undecided {} (fails the gate): {}",
        unit.path.display(),
        unit.line,
        unit.rule,
        sentence(&unit.what, WHY_CHARS)
    )
}

/// What keeps the agent working, `where` they are: "2 findings in this
/// turn's changes are not fixed or dismissed", "1 undecided unit in this
/// turn's changes fails the quality gate", or both.
fn open_things(findings: usize, undecided: usize, place: &str) -> String {
    let open = |n: usize, place: &str| {
        format!(
            "{}{place} {} not fixed or dismissed",
            output::count(n, "finding"),
            if n == 1 { "is" } else { "are" }
        )
    };
    let unsure = |n: usize, place: &str| {
        format!(
            "{}{place} {} the quality gate",
            output::count(n, "undecided unit"),
            if n == 1 { "fails" } else { "fail" }
        )
    };
    match (findings, undecided) {
        (_, 0) => open(findings, place),
        (0, _) => unsure(undecided, place),
        _ => format!("{}, and {}", open(findings, place), unsure(undecided, "")),
    }
}

/// The agent's note on edited files of code the check did not judge, so
/// that silence about them is not read as a pass.
fn unreviewed_agent(files: &[&Unreviewed]) -> Option<String> {
    let line = |file: &Unreviewed| format!("{}: {}.", file.path.display(), reason(&file.why));
    match files {
        [] => None,
        [one] => Some(format!("JevGate did not review {}", line(one))),
        _ => {
            let mut text = format!(
                "JevGate did not review {}:",
                output::count(files.len(), "edited file")
            );
            for file in files.iter().take(SHOWN_GUARDS) {
                text.push_str(&format!("\n- {}", line(file)));
            }
            if files.len() > SHOWN_GUARDS {
                text.push_str(&format!(
                    "\n{} not shown.",
                    output::count(files.len() - SHOWN_GUARDS, "more")
                ));
            }
            Some(text)
        }
    }
}

/// The person's note on changed files of code the turn's check did not
/// judge, each with the first clause of why.
pub(super) fn unreviewed_user(files: &[Unreviewed]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let named = first_named(files, NAMED_GUARDS, |file| {
        let why = file.why.split(['.', ';', ',']).next().unwrap_or_default();
        format!("{} ({})", file.path.display(), why.trim())
    });
    Some(format!(
        "JevGate did not review {} this turn changed: {named}.",
        output::count(files.len(), "file"),
    ))
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
        fail(known.iter().filter(|f| f.fails()).count())
    );
    match (new.is_empty(), known.is_empty()) {
        (true, true) => return None,
        (true, false) => return Some(format!("{reviewed} {earlier}")),
        _ => {}
    }
    let failing = new.iter().filter(|f| f.fails()).count();
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
    let all = new.len() + known.len();
    let blocks = match new.iter().chain(known).filter(|f| f.blocks()).count() {
        0 => "None of them blocks the end of the turn.",
        n if n == all => {
            "Each one blocks the end of the turn until it is fixed or dismissed with a reason."
        }
        _ => {
            "Each one not marked optional blocks the end of the turn until it is fixed or dismissed with a reason."
        }
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
/// A `carried` turn began where one JevGate could not check did, so its
/// changes are those since JevGate last checked.
pub(super) fn block_reason(
    open: &[Flagged],
    undecided: &[Undecided],
    block: u32,
    carried: bool,
) -> String {
    let place = if carried {
        " in code changed since JevGate last checked"
    } else {
        " in code changed this turn"
    };
    let head = format!(
        "JevGate blocked the end of this turn ({block} of at most {MAX_BLOCKS}): {}.",
        open_things(open.len(), undecided.len(), place)
    );
    let mut tail: String = undecided
        .iter()
        .take(SHOWN)
        .map(|unit| format!("{}\n", undecided_line(unit)))
        .collect();
    if !undecided.is_empty() {
        tail.push_str(UNDECIDED_NEXT);
        tail.push(' ');
    }
    if !open.is_empty() {
        let each = if open.len() == 1 { "It" } else { "Each one" };
        tail.push_str(&format!(
            "{each} is a place worth a look: read the code there and fix it when it is right; when it is mistaken, intended or right but left for later, {DISMISS}. "
        ));
    }
    tail.push_str("JevGate does not block again when nothing changed.");
    if open.iter().any(|f| f.accepted_this_turn) {
        tail.push_str(" A `jevgate: allow` comment or a baseline entry without a reason added this turn counts from the next turn: accepting findings that way is the person's call, so dismiss with a reason instead.");
    }
    list(&head, open, &tail, MAX_CHARS)
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
    text.push_str("JevGate reports these to the person at the end of the turn. Within a turn it reads jevgate.toml, custom questions, the baseline and `jevgate: allow` comments as they were when the turn began, except a finding dismissed with a reason through `jevgate baseline mark`.");
    Some(text)
}

/// The person's note on what a turn did to the checks around the code, and
/// what the gate read when the turn edited them.
pub(super) fn guards_user(guards: &[Guard]) -> Option<String> {
    if guards.is_empty() {
        return None;
    }
    let named = first_named(guards, NAMED_GUARDS, |g| clip(&g.describe(), GUARD_CHARS));
    let mut text = format!("JevGate: this turn {} ({named}).", guards::summary(guards));
    let edited = |g: &Guard| {
        matches!(
            g.kind,
            Kind::Allow | Kind::Configuration | Kind::Question | Kind::Baseline
        )
    };
    if guards.iter().any(edited) {
        text.push_str(" Its gate read jevgate.toml, custom questions, the baseline and `jevgate: allow` comments as they were when the turn began, except findings dismissed with a reason.");
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

/// The person's note on a block, on `open` findings and `undecided` units
/// that fail the gate.
pub(super) fn blocked(open: usize, undecided: usize, block: u32) -> String {
    let them = if open + undecided == 1 { "it" } else { "them" };
    let asked = if open > 0 {
        format!("fix or dismiss {them} with a reason")
    } else {
        format!("fix {them}")
    };
    format!(
        "JevGate: {}; the agent is asked to {asked} (block {block} of {MAX_BLOCKS}).",
        open_things(open, undecided, " in this turn's changes")
    )
}

/// Why a stop with open findings lets the agent finish.
pub(super) enum LetThrough {
    /// The turn was blocked as often as the hook blocks one.
    Cap,
    /// Nothing changed since the last block.
    Unchanged,
}

/// Why a stop with `open` findings and `undecided` units that fail the
/// gate lets the agent finish.
pub(super) fn let_through(open: usize, undecided: usize, why: LetThrough) -> String {
    let findings = (open > 0).then(|| {
        format!(
            "{} {} still not fixed or dismissed",
            output::count(open, "finding"),
            if open == 1 { "is" } else { "are" }
        )
    });
    let units = (undecided > 0).then(|| {
        format!(
            "{} still {} the quality gate",
            output::count(undecided, "undecided unit"),
            if undecided == 1 { "fails" } else { "fail" }
        )
    });
    let still = joined(findings, units, ", and ").unwrap_or_default();
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
/// are fixed or dismissed, the findings the agent dismissed with a reason,
/// and those optional here.
pub(super) fn passed(
    after_block: bool,
    (dismissed, optional): (&[Dismissed], &[Flagged]),
    unreviewed: bool,
) -> Option<String> {
    let mut notes = Vec::new();
    if after_block {
        // A blocked unit the parser can no longer read is not fixed.
        notes.push(if unreviewed {
            "JevGate: no finding of this turn is open now, but some of the code it changed was not reviewed.".to_string()
        } else {
            "JevGate: the findings that blocked this turn are fixed or dismissed.".to_string()
        });
    }
    if !dismissed.is_empty() {
        let named = first_named(dismissed, NAMED_DISMISSED, |d| {
            let note = d.note.as_ref().map_or(String::new(), |n| format!(" ({n})"));
            format!(
                "{}:{} {} as {}{note}",
                d.path.display(),
                d.line,
                d.rule,
                output::label(&d.reason)
            )
        });
        notes.push(format!(
            "JevGate: the agent dismissed {} in this turn's changes ({named}); `jevgate baseline list` lists them and `jevgate baseline stats` counts them by rule and reason.",
            output::count(dismissed.len(), "finding"),
        ));
    }
    if !optional.is_empty() {
        notes.push(format!(
            "JevGate: {} in this turn's changes {} optional here. {LIST_THEM}",
            output::count(optional.len(), "finding"),
            if optional.len() == 1 { "is" } else { "are" }
        ));
    }
    (!notes.is_empty()).then(|| notes.join(" "))
}

/// What the agent reads when a session starts: the instructions `init
/// --agent` writes quote it, and ask an agent that never read it (hooks not
/// trusted yet, an agent that reads the instructions but not the hooks) to
/// check its changes itself.
pub(super) const RUNNING: &str =
    "JevGate's hooks run in this session: they check each edit and the end of each turn.";

/// The person's note on a stop with no record of the turn's start.
pub(super) const UNCHECKED_TURN: &str = "JevGate did not check this turn: it has no snapshot of the turn's start, so its prompt hook may not be installed. It checks from the next turn on.";

/// What the person is told when `what` could not be checked.
pub(super) fn failed_user(what: &str, why: &str) -> String {
    format!(
        "JevGate could not check {what}: {}. Nothing was blocked.",
        reason(why)
    )
}

/// Why a check the hook ran while waiting out a provider failure could not
/// finish: it asked nothing, and the cache did not hold every answer.
pub(crate) fn waiting(outage: &super::outage::Outage) -> String {
    let minutes = outage.minutes_left();
    format!(
        "the provider failed a few minutes ago ({}), so JevGate asks it again in {} and uses only cached answers until then, which did not cover this",
        reason(&outage.reason),
        output::count(minutes as usize, "minute")
    )
}

/// What the agent is told at its next turn when the end of the last one
/// could not be checked.
pub(super) fn unchecked_turn(why: &str) -> String {
    format!(
        "JevGate could not check the last turn's changes ({}), so they were not reviewed; this is not a pass. JevGate checks them with this turn's changes when it ends.",
        reason(why)
    )
}

/// What the person is told when a turn's start holds a configuration that
/// does not load: its changes stay unchecked, and the next turn starts
/// where this one ended.
pub(super) fn unreadable_start_user(why: &str) -> String {
    format!(
        "JevGate could not check this turn: {}. Nothing was blocked, and this turn's changes stay unchecked; JevGate checks the next turn from where this one ended, once its configuration loads.",
        reason(why)
    )
}

/// What the agent is told at its next event about a turn whose start held
/// a configuration that does not load.
pub(super) fn unreadable_start_agent(why: &str) -> String {
    format!(
        "JevGate could not check the last turn's changes ({}), so they were not reviewed; this is not a pass.",
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

/// The first `most` of `items` as `name` writes them, and how many more
/// there are, separated by semicolons.
fn first_named<T>(items: &[T], most: usize, name: impl Fn(&T) -> String) -> String {
    let mut named: Vec<String> = items.iter().take(most).map(name).collect();
    if items.len() > most {
        named.push(format!("{} more", items.len() - most));
    }
    named.join("; ")
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

/// `- path:line level rule (fails the gate): why Right 87% of the time (23
/// labels). Next: step`, from the fields the MCP tools return for the same
/// finding. A long why is cut, never how often such findings were right.
fn line(flagged: &Flagged) -> String {
    let finding = FindingView::new(&flagged.path, &flagged.finding);
    let mark = match (
        finding.fails(),
        flagged.accepted_this_turn,
        flagged.blocks(),
    ) {
        (true, true, _) => " (fails the gate; accepted this turn)",
        (true, false, _) => " (fails the gate)",
        (false, true, _) => " (accepted this turn)",
        (false, false, true) => "",
        (false, false, false) => " (optional)",
    };
    let why = sentence(&flagged.finding.message, WHY_CHARS);
    format!(
        "- {}:{} {} {}{mark}: {} Next: {}",
        finding.path.display(),
        finding.line,
        output::label(&finding.strength),
        finding.rule,
        output::claimed(&why, &flagged.path, &flagged.finding, output::Style::PLAIN),
        sentence(finding.action, NEXT_CHARS)
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
