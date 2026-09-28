//! What the hook says: one line per finding for the agent, within what every
//! agent reads whole, and short notes for the person. Context states facts;
//! only the reason of a block, which the agent is meant to act on, instructs.
use super::review::{Flagged, Undecided, Unreviewed};
use crate::{
    guards::{self, Guard, Kind},
    output,
    schema::Strength,
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

/// What fails the gate, for a sentence: "2 findings", "1 undecided unit",
/// "1 finding and 2 undecided units".
fn failing_things(findings: usize, undecided: usize) -> String {
    match (findings, undecided) {
        (_, 0) => output::count(findings, "finding"),
        (0, _) => output::count(undecided, "undecided unit"),
        _ => format!(
            "{} and {}",
            output::count(findings, "finding"),
            output::count(undecided, "undecided unit")
        ),
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
    let mut named: Vec<String> = files
        .iter()
        .take(NAMED_GUARDS)
        .map(|file| {
            let why = file.why.split(['.', ';', ',']).next().unwrap_or_default();
            format!("{} ({})", file.path.display(), why.trim())
        })
        .collect();
    if files.len() > NAMED_GUARDS {
        named.push(format!("{} more", files.len() - NAMED_GUARDS));
    }
    Some(format!(
        "JevGate did not review {} this turn changed: {}.",
        output::count(files.len(), "file"),
        named.join("; ")
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
    let blocks = if failing > 0 || known.iter().any(Flagged::fails) {
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
/// A `carried` turn began where one JevGate could not check did, so its
/// changes are those since JevGate last checked.
pub(super) fn block_reason(
    failing: &[Flagged],
    undecided: &[Undecided],
    block: u32,
    carried: bool,
) -> String {
    let one = failing.len() + undecided.len() == 1;
    let head = format!(
        "JevGate blocked the end of this turn ({block} of at most {MAX_BLOCKS}): {} in code changed {} {} the quality gate.",
        failing_things(failing.len(), undecided.len()),
        if carried {
            "since JevGate last checked"
        } else {
            "this turn"
        },
        if one { "fails" } else { "fail" }
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
    let (them, a_finding) = if failing.len() == 1 {
        ("it", "it")
    } else {
        ("them", "a finding")
    };
    if failing.is_empty() {
        tail.push_str("JevGate does not block again when nothing changed.");
    } else {
        tail.push_str(&format!(
            "Fix {them}, then finish. If {a_finding} is mistaken, keep the code as it is and say why in your reply; JevGate does not block again when nothing changed."
        ));
    }
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
    text.push_str("JevGate reports these to the person at the end of the turn. Within a turn it reads jevgate.toml, custom questions, the baseline and `jevgate: allow` comments as they were when the turn began.");
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
    let edited = |g: &Guard| {
        matches!(
            g.kind,
            Kind::Allow | Kind::Configuration | Kind::Question | Kind::Baseline
        )
    };
    if guards.iter().any(edited) {
        text.push_str(" Its gate read jevgate.toml, custom questions, the baseline and `jevgate: allow` comments as they were when the turn began.");
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

/// The person's note on a block, on `failing` findings and `undecided`
/// units that fail the gate.
pub(super) fn blocked(failing: usize, undecided: usize, block: u32) -> String {
    let one = failing + undecided == 1;
    format!(
        "JevGate: {} in this turn's changes {} the quality gate; the agent is asked to fix {} (block {block} of {MAX_BLOCKS}).",
        failing_things(failing, undecided),
        if one { "fails" } else { "fail" },
        if one { "it" } else { "them" }
    )
}

/// Why a stop with findings that fail the gate lets the agent finish.
pub(super) enum LetThrough {
    /// The turn was blocked as often as the hook blocks one.
    Cap,
    /// Nothing changed since the last block.
    Unchanged,
}

/// Why a stop with `failing` findings and `undecided` units that fail the
/// gate lets the agent finish.
pub(super) fn let_through(failing: usize, undecided: usize, why: LetThrough) -> String {
    let still = format!(
        "{} still {} the quality gate",
        failing_things(failing, undecided),
        if failing + undecided == 1 {
            "fails"
        } else {
            "fail"
        }
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
pub(super) fn waiting(outage: &super::outage::Outage) -> String {
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
    let mark = match (finding.fails(), flagged.accepted_this_turn) {
        (true, true) => " (fails the gate; accepted this turn)",
        (true, false) => " (fails the gate)",
        (false, true) => " (accepted this turn)",
        (false, false) => "",
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
