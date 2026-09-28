//! The characters a question's text may hold and a proposal may quote: its
//! text reaches terminals, CI logs and the rules table as written.

/// Unicode's bidirectional controls: invisible, and able to make a line
/// read differently on a terminal than it is.
const BIDI_CONTROLS: [char; 9] = [
    '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}',
    '\u{2069}',
];

/// Unicode's line and paragraph separators, where some terminals break a
/// line and TOML does not.
const SEPARATORS: [char; 2] = ['\u{2028}', '\u{2029}'];

/// Invisible at the start of a file, which some editors write.
pub const BYTE_ORDER_MARK: char = '\u{feff}';

/// Whether a character is kept in what is quoted or shown: no control
/// character, which a TOML comment cannot hold and a terminal acts on, no
/// line or paragraph separator, and no bidirectional control or byte-order
/// mark, which are invisible.
pub fn printable(c: char) -> bool {
    !c.is_control()
        && !BIDI_CONTROLS.contains(&c)
        && !SEPARATORS.contains(&c)
        && c != BYTE_ORDER_MARK
}

/// The first character of `text` a question may not hold: one that is not
/// [`printable`], except the line breaks and tabs of a `multiline` text.
pub(super) fn unprintable(text: &str, multiline: bool) -> Option<char> {
    text.chars()
        .find(|&c| !printable(c) && !(multiline && matches!(c, '\n' | '\r' | '\t')))
}
