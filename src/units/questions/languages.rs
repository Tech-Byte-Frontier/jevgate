//! Examples a follow-up question adds about one language's files, where a
//! language's libraries or framework decide the answer and their names do
//! not say so. Only follow-up checks are reworded: the broad questions that
//! lead to them stay general, so a file's other cached answers stay valid.
use super::csharp::add_examples;
use serde_json::Value;

/// The languages whose files these examples reword, as a file's state names them.
pub const RUST: &str = "Rust";
pub const RUBY: &str = "Ruby";

/// A language, a question id, then examples of "yes" and of "no".
type Examples = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
);

/// zero2prod's subscription token, taken from `thread_rng()`, read as made
/// with a non-cryptographic generator. Lobsters' controller actions that
/// look records up with `find_by!` stayed undecided on whether they send an
/// exception's text to clients: Rails answers a raised RecordNotFound with
/// its 404 page, and other exceptions with its 500 page, without the text.
const EXAMPLES: [Examples; 2] = [
    (
        RUST,
        "random",
        &[
            "SmallRng, fastrand, or a generator seeded with a fixed number or the time, used for a secret value",
        ],
        &[
            "rand's thread_rng() or rand::rng(), or a StdRng or ChaCha generator seeded from entropy or OsRng: these are cryptographically secure",
        ],
    ),
    (
        RUBY,
        "exception_to_client",
        &["Rendering `e.message` or `e.backtrace` of a rescued exception into the response"],
        &[
            "Exceptions it lets propagate, such as ActiveRecord::RecordNotFound from `find` or `find_by!`: Rails answers them with its 404 or 500 page, without their text, in production",
        ],
    ),
];

pub fn reword_language(language: &str, id: &str, body: &mut Value) {
    let Some((.., yes, no)) = EXAMPLES
        .iter()
        .find(|(lang, question, ..)| *lang == language && *question == id)
    else {
        return;
    };
    if body["type"] == "score" {
        return;
    }
    add_examples(&mut body["criteria"]["true"], yes);
    add_examples(&mut body["criteria"]["false"], no);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::questions;

    #[test]
    fn follow_up_checks_name_a_language_s_libraries_and_other_languages_keep_their_wording() {
        let check = |id: &str| {
            questions::WEAK_SETTINGS
                .iter()
                .chain(&questions::EXPOSURES)
                .find(|c| c.id == id)
                .unwrap()
                .body("function.source")
        };
        let general = check("random");
        let mut rust = general.clone();
        reword_language(RUST, "random", &mut rust);
        let no = rust["criteria"]["false"]["examples"].to_string();
        assert!(no.contains("thread_rng"), "{no}");
        let mut python = general.clone();
        reword_language("Python", "random", &mut python);
        assert_eq!(python, general);
        let mut ruby = check("exception_to_client");
        reword_language(RUBY, "exception_to_client", &mut ruby);
        assert!(
            ruby["criteria"]["false"]["examples"]
                .to_string()
                .contains("RecordNotFound")
        );
        let mut weakened = questions::security_weakened("function.source", false);
        let before = weakened.clone();
        reword_language(RUST, "weakened", &mut weakened);
        assert_eq!(weakened, before, "broad questions stay general");
    }
}
