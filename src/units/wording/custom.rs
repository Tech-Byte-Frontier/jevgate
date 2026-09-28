//! The message of a custom question's finding: the unit, the question and
//! the answer; its action is the next step the question's author wrote.
use super::Wording;
use crate::{
    custom::{Kind, Question},
    units::{comments::TOP_LEVEL, instructions::PREAMBLE},
};

/// `` `charge`: Does this function log a request body? Yes. ``
pub(in crate::units) fn custom_wording(question: &'static Question, name: &str) -> Wording {
    let subject = match (question.unit, name) {
        (Kind::Comment, TOP_LEVEL) => "A comment outside every definition".to_string(),
        (Kind::Comment, _) => format!("A comment in `{name}`"),
        (Kind::Section, PREAMBLE) => "The text before the first heading".to_string(),
        (Kind::Section, _) => format!("Section `{name}`"),
        (Kind::File, _) => "This file".to_string(),
        (Kind::Hunk, _) => format!("The change at {name}"),
        (Kind::Function | Kind::Test, _) => format!("`{name}`"),
    };
    (
        format!("{subject}: {} Yes.", question.question),
        question.next_step.as_str(),
    )
}
