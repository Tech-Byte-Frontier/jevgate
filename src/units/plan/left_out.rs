//! What syntax errors left out of a file, as the report names it, and the
//! outline they leave out when too little of the file parsed to describe it.
use crate::{analysis::units::FileUnits, schema::LeftOut, units::FilePlan};

/// The share of a file's non-blank lines its parse must cover for its
/// outline to be asked: an outline lists every member as the file's, and
/// one missing its broken members describes another file. Leaving out whole
/// members of 55 clean outlines on tuned projects and asking again (183
/// first answers against the whole file's): with 90% or more parsed, most
/// often one small member missing, the split answer's top level moved 0.03
/// on average and 46 of 48 kept their finding or its absence; from 70% to
/// 90% it moved twice as much and 10 of 99 flipped; below 70%, 0.10 to 0.18.
pub(super) const OUTLINE_COVERAGE: f64 = 0.9;

/// The report's entry for each unit and each run of code outside every
/// unit that syntax errors left out. It names the parser, not the code:
/// nearly every error the corpus and the projects measured for 0.30 hold is
/// valid code a grammar lacks (Swift's `x as? T ?? y`, Kotlin 2's backing
/// fields, Bend 2's erased binders), and a coding agent told "syntax
/// error" edits code that is correct.
pub(super) fn entries(parsed: &FileUnits, source: &str, language: &str) -> Vec<LeftOut> {
    parsed
        .left_out_code(source)
        .into_iter()
        .map(|l| LeftOut {
            unit: l.name,
            start_line: l.line,
            end_line: l.end_line,
            reason: format!(
                "The {language} parser could not read line {}.",
                l.error_line
            ),
        })
        .collect()
}

/// Whether the parse covers enough of a file to ask its outline; when it
/// does not, the outline is recorded as left out with the share it covers.
pub(super) fn outline_covered(parsed: &FileUnits, source: &str, file: &mut FilePlan) -> bool {
    let coverage = parsed.coverage(source);
    if coverage >= OUTLINE_COVERAGE {
        return true;
    }
    file.left_out.push(LeftOut {
        unit: "outline".into(),
        start_line: 1,
        end_line: source.lines().count().max(1),
        reason: format!(
            "Only {:.0}% of the file's lines parsed; an outline needs {:.0}%.",
            (coverage * 100.0).floor(),
            OUTLINE_COVERAGE * 100.0
        ),
    });
    false
}
