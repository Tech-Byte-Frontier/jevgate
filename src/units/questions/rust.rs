//! How questions read about Rust files. The `rand` crate's default
//! generators are cryptographically secure, which its names do not say:
//! zero2prod's subscription token, taken from `thread_rng()`, read as made
//! with a non-cryptographic generator. These examples extend the general
//! criteria only in requests about Rust files.
use super::csharp::add_examples;
use serde_json::Value;

/// The language whose files these additions reword, as a file's state names it.
pub const RUST: &str = "Rust";

const SECURE_RAND: &str = "rand's thread_rng() or rand::rng(), or a StdRng or ChaCha generator seeded from entropy or OsRng: these are cryptographically secure";

/// Examples a question adds about Rust files: its id, then examples of "yes"
/// and of "no".
/// Only the follow-up check is reworded: the broad question that leads to
/// it stays general, so a Rust file's other cached answers stay valid.
const EXAMPLES: [(&str, &[&str], &[&str]); 1] = [(
    "random",
    &[
        "SmallRng, fastrand, or a generator seeded with a fixed number or the time, used for a secret value",
    ],
    &[SECURE_RAND],
)];

pub fn reword_rust(language: &str, id: &str, body: &mut Value) {
    if language != RUST {
        return;
    }
    let Some((_, yes, no)) = EXAMPLES.iter().find(|(question, ..)| *question == id) else {
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
    fn rust_files_name_rand_s_secure_generators_and_other_languages_keep_their_wording() {
        let general = questions::WEAK_SETTINGS
            .iter()
            .find(|c| c.id == "random")
            .unwrap()
            .body("function.source");
        let mut rust = general.clone();
        reword_rust(RUST, "random", &mut rust);
        let no = rust["criteria"]["false"]["examples"].to_string();
        assert!(no.contains("thread_rng"), "{no}");
        let mut python = general.clone();
        reword_rust("Python", "random", &mut python);
        assert_eq!(python, general);
    }
}
