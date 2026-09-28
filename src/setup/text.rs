//! Text JevGate keeps in agents' instruction files: a block between markers
//! in a file others write too (`AGENTS.md`, `GEMINI.md`), or a whole file
//! of its own where the agent reads a directory of rules (Claude Code,
//! Cursor) or plugins (OpenCode).
use anyhow::{Result, bail};
use std::ops::Range;

/// What JevGate's findings mean to the agent, between its markers.
pub(super) const INSTRUCTIONS: &str = include_str!("instructions.md");
/// The start of the line that opens JevGate's block; the rest of the line
/// tells a reader where the block comes from.
const BEGIN: &str = "<!-- jevgate:begin";
const END: &str = "<!-- jevgate:end -->";
/// Whole files JevGate owns carry one of these; a file without them is
/// someone else's, and is never overwritten or removed.
const OWNED: [&str; 2] = ["jevgate:begin", "jevgate:managed"];

/// `text` with JevGate's block in place of the old one, or after a blank
/// line at the end, in the file's own line ends.
pub(super) fn with_block(text: &str, block: &str) -> Result<String> {
    let newline = newline(text);
    let block = block.replace('\n', newline);
    Ok(match find(text)? {
        Some(range) => format!("{}{block}{}", &text[..range.start], &text[range.end..]),
        None if text.trim_start_matches('\u{feff}').trim().is_empty() => block,
        None => {
            let ending = if text.ends_with('\n') { "" } else { newline };
            format!("{text}{ending}{newline}{block}")
        }
    })
}

/// `text` without JevGate's block and the blank line written before it;
/// `None` when nothing else is left, so the file can go.
pub(super) fn without_block(text: &str) -> Result<Option<String>> {
    let Some(range) = find(text)? else {
        return Ok(Some(text.to_string()));
    };
    let newline = newline(text);
    let before = &text[..range.start];
    let blank = format!("{newline}{newline}");
    let before = if before.ends_with(&blank) {
        &before[..before.len() - newline.len()]
    } else {
        before
    };
    let rest = format!("{before}{}", &text[range.end..]);
    Ok((!rest.trim_start_matches('\u{feff}').trim().is_empty()).then_some(rest))
}

/// Whether `text` is a file JevGate wrote whole.
pub(super) fn owned(text: &str) -> bool {
    OWNED.iter().any(|marker| text.contains(marker))
}

/// The bytes of JevGate's block, from the start of its first line through
/// the end of its last, or none. Markers out of pairs are an error: which
/// text is JevGate's would be a guess.
fn find(text: &str) -> Result<Option<Range<usize>>> {
    let mut begin = None;
    let mut found = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let marker = line.trim();
        if marker.starts_with(BEGIN) {
            if begin.is_some() || found.is_some() {
                bail!("it has more than one JevGate block");
            }
            begin = Some(at);
        } else if marker.starts_with(END) {
            let Some(start) = begin.take() else {
                bail!("it has a JevGate end marker without its begin marker");
            };
            found = Some(start..at + line.len());
        }
        at += line.len();
    }
    if begin.is_some() {
        bail!("it has a JevGate begin marker without its end marker ({END})");
    }
    Ok(found)
}

/// The file's line end: `\r\n` when it uses them.
fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = "<!-- jevgate:begin (test) -->\nBe kind.\n<!-- jevgate:end -->\n";

    #[test]
    fn a_block_is_added_replaced_and_removed_back_to_the_original() {
        let original = "# Rules\n\nRun the tests.\n";
        let added = with_block(original, BLOCK).unwrap();
        assert_eq!(added, format!("{original}\n{BLOCK}"));
        assert_eq!(with_block(&added, BLOCK).unwrap(), added, "unchanged");
        let newer = BLOCK.replace("kind", "brief");
        let replaced = with_block(&added, &newer).unwrap();
        assert_eq!(replaced, format!("{original}\n{newer}"));
        assert_eq!(without_block(&replaced).unwrap().as_deref(), Some(original));
        assert!(find(&replaced).unwrap().is_some() && find(original).unwrap().is_none());
    }

    #[test]
    fn a_file_left_empty_goes_and_text_after_the_block_stays() {
        let created = with_block("", BLOCK).unwrap();
        assert_eq!(created, BLOCK);
        assert_eq!(without_block(&created).unwrap(), None);
        let edited = format!("Intro.\n\n{BLOCK}\nMore.\n");
        assert_eq!(
            without_block(&edited).unwrap().as_deref(),
            Some("Intro.\n\nMore.\n")
        );
        let unended = with_block("No newline.", BLOCK).unwrap();
        assert_eq!(unended, format!("No newline.\n\n{BLOCK}"));
    }

    #[test]
    fn line_ends_follow_the_file() {
        let original = "# Rules\r\n\r\nRun the tests.\r\n";
        let added = with_block(original, BLOCK).unwrap();
        assert!(!added.replace("\r\n", "").contains('\n'), "{added:?}");
        assert_eq!(without_block(&added).unwrap().as_deref(), Some(original));
    }

    #[test]
    fn markers_out_of_pairs_are_refused() {
        for text in [
            "<!-- jevgate:begin -->\nText.\n",
            "Text.\n<!-- jevgate:end -->\n",
            &format!("{BLOCK}{BLOCK}"),
        ] {
            assert!(with_block(text, BLOCK).is_err(), "{text:?}");
            assert!(without_block(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn the_instructions_are_one_block() {
        assert_eq!(find(INSTRUCTIONS).unwrap(), Some(0..INSTRUCTIONS.len()));
        assert!(owned(INSTRUCTIONS));
        assert!(!owned("# Someone else's rules\n"));
    }
}
