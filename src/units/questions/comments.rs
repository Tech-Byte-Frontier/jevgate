//! Questions about one code comment and the code it is about. Each concern is
//! its own question: repeating the code, more words than the point needs,
//! narrating an edit, and code turned off.
use super::{EVIDENCE, noul, score};
use serde_json::{Map, Value, json};

/// The note of every comment question: what the state holds for it.
fn note(path: &str, source: bool) -> String {
    let source = if source {
        " `function_source` holds the whole definition the comment sits in."
    } else {
        ""
    };
    format!(
        "`{path}.code` holds the code the comment is about and `{path}.placement` where the comment sits.{source}"
    )
}

/// Whether the comment only repeats its code. A description of what a
/// function returns or guarantees beyond its name and signature is not
/// repetition, nor is what a flag, pattern or value stands for: asked
/// without them, Pygments' `# class: 'err'` beside a token's color and
/// yt-dlp's `"-x",  # Extract audio` read as repeating their line. A heading
/// over a group of lines is the middle level, since whether it helps a
/// reader skim is a matter of taste.
pub fn comment_restates(path: &str, source: bool) -> Value {
    score(
        format!("Does the comment in `{path}.text` only repeat what `{path}.code` already says?"),
        &note(path, source),
        [
            "No. It tells a reader something the code does not say: why the code is written this way; a constraint, caveat, unit or requirement; what a flag, option, pattern, number or term means or stands for, or a value in another unit, such as `14px` beside `0.875rem`; what a step achieves when the code does not make it obvious; or how to use a definition beyond its name and signature.",
            "Partly. It repeats the code but adds a detail a reader could use, or it is a heading that names the group of lines or definitions below it.",
            "Yes. A reader learns nothing from it that the names, calls and values in the code do not already say, such as `increment the counter` above `count += 1`, `Parse the file` above `parse_file(path)` or `Returns the user` above `get_user()`.",
        ],
    )
}

/// Whether sentences of the comment add nothing. Asked whether it "could say
/// the same in far fewer words", every multi-line JSDoc block explaining a
/// rounding rule, a sort cycle or a matching strategy was a yes. Parameter
/// entries are named by whether the signature already writes their types:
/// the Sphinx `:param` and `:rtype:` entries of psf/requests' untyped
/// functions read as sentences that add nothing.
pub fn comment_verbose(path: &str, source: bool) -> Value {
    score(
        format!(
            "Could sentences of the comment in `{path}.text` be removed without losing anything it tells a reader?"
        ),
        &note(path, source),
        [
            "No. Each sentence adds a reason, fact, caveat, example, step or entry a reader needs, even when the comment is long, such as a strategy, a rounding rule, or parameter and return entries (`:param`, `Args:`, `@param`) that give a type or meaning the signature in `code` does not write.",
            "Slightly. One phrase or sentence adds nothing.",
            "Yes. Several sentences add nothing: they say what another sentence already says, state what any code of its kind does, or are parameter and return entries that only repeat a name, or a type the signature in `code` already writes, such as `numerator: The numerator value.` or `user_id (int): The user id.` for `user_id: int`.",
        ],
    )
}

/// Points at the code, since a heading such as `Fixed` above a list of fixed
/// costs read as a fix without it.
pub fn comment_history(path: &str) -> Value {
    with_code(
        path,
        noul(
            format!(
                "Does the comment in `{path}.text` describe an edit made to the code, such as what was added, changed, fixed, moved or removed, instead of the code as it is?"
            ),
            "It says what the code was before or that it was changed: added, replaced, fixed, moved or updated, such as `now uses the cache instead of the database`, `changed to async`, `added validation` or `as requested in review`.",
            "It describes the code as it is now, including headings and category names, such as `Fixed` above a list of fixed costs. A reason that names a past bug, incident or version, such as why a check exists, still describes the code.",
        ),
    )
}

/// A question whose note also says where the comment's code is.
fn with_code(path: &str, mut question: Value) -> Value {
    question["instructions"]["note"] = json!(format!("{} {EVIDENCE}", note(path, false)));
    question
}

pub fn comment_disabled(path: &str) -> Value {
    noul(
        format!("Is the comment in `{path}.text` code turned off by commenting it out?"),
        "Most of it is statements, declarations or markup that would run if the comment markers were removed.",
        "It is prose, or shows code as an example of how to call or use something.",
    )
}

/// Kinds of comments whose whole content a reader could do without; every
/// other kind tells the reader something.
pub const CONCERN_KINDS: [&str; 4] = ["restates", "narration", "history", "disabled"];

/// What a comment tells a reader, as a Choice among kinds of comments, asked
/// in a request of its own when its questions stay undecided.
pub fn comment_kind(source: bool) -> Value {
    let kinds: [(&str, &str); 10] = [
        (
            "reason",
            "Why the code is written this way: a reason, constraint, trade-off, workaround or decision.",
        ),
        (
            "caveat",
            "A warning, invariant, limitation, requirement or unit the code relies on, or work left to do.",
        ),
        (
            "reference",
            "A link, ticket, specification, algorithm or source the code follows.",
        ),
        (
            "usage",
            "How to use a definition: what it returns, accepts, guarantees or is for, beyond its name and signature.",
        ),
        (
            "summary",
            "What a block of several steps achieves, in fewer words than its code, where the code does not make it obvious at a glance.",
        ),
        (
            "label",
            "A heading that names a section of the file or a group of definitions.",
        ),
        (
            "restates",
            "What the code next to it does, in the terms the code already uses: its names, calls and values.",
        ),
        (
            "narration",
            "A step-by-step account of what the code does, longer than the point it makes.",
        ),
        (
            "history",
            "An edit made to the code: what was added, changed, fixed, moved or removed.",
        ),
        ("disabled", "Code turned off by commenting it out."),
    ];
    let source = if source {
        " `function_source` holds the whole definition the comment sits in."
    } else {
        ""
    };
    json!({
        "type": "choice",
        "instructions": {
            "question": "Which best describes what the comment in `comment.text` tells a reader?",
            "note": format!("`comment.code` holds the code the comment is about and `comment.placement` where the comment sits.{source} {EVIDENCE}"),
        },
        "criteria": kinds.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_, _>>(),
    })
}
