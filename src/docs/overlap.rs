//! Candidate pairs of sections that may repeat each other across documents:
//! the share of the smaller section's three-word sequences found in the
//! other. The share only selects pairs to ask about; it never decides.
use std::collections::{BTreeMap, BTreeSet};

/// A pair is a candidate when this share of the smaller section recurs.
pub const MIN_SHARE: f64 = 0.3;
/// Sections with fewer three-word sequences are too short to compare.
const MIN_SHINGLES: usize = 15;
/// Caps bound what is kept, not what is sent: pairs inside a finished plan
/// are never asked, so one plan and its design cannot use up the run.
const RUN_CAP: usize = 1_000;
/// Pairs kept between any two documents.
const FILE_PAIR_CAP: usize = 3;

/// One section to compare: its file and text.
pub struct Text<'a> {
    pub file: usize,
    pub text: &'a str,
}

fn shingles(text: &str) -> BTreeSet<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    words.windows(3).map(|w| w.join(" ")).collect()
}

/// Candidate pairs as indexes into `texts`, most shared first, with the
/// number omitted by the caps.
pub fn pairs(texts: &[Text<'_>]) -> (Vec<(usize, usize, f64)>, usize) {
    let sets: Vec<BTreeSet<String>> = texts.iter().map(|t| shingles(t.text)).collect();
    let mut index = BTreeMap::<&str, Vec<usize>>::new();
    for (i, set) in sets.iter().enumerate() {
        if set.len() >= MIN_SHINGLES {
            for s in set {
                index.entry(s).or_default().push(i);
            }
        }
    }
    let mut shared = BTreeMap::<(usize, usize), usize>::new();
    for owners in index.values() {
        for (n, &a) in owners.iter().enumerate() {
            for &b in &owners[n + 1..] {
                if texts[a].file != texts[b].file {
                    *shared.entry((a, b)).or_default() += 1;
                }
            }
        }
    }
    let mut found: Vec<(usize, usize, f64)> = shared
        .into_iter()
        .map(|((a, b), n)| (a, b, n as f64 / sets[a].len().min(sets[b].len()) as f64))
        .filter(|(_, _, share)| *share >= MIN_SHARE)
        .collect();
    found.sort_by(|x, y| y.2.total_cmp(&x.2).then((x.0, x.1).cmp(&(y.0, y.1))));
    let mut per_files = BTreeMap::<(usize, usize), usize>::new();
    let mut kept = Vec::new();
    let mut omitted = 0;
    for pair in found {
        let (fa, fb) = (texts[pair.0].file, texts[pair.1].file);
        let full = kept.len() >= RUN_CAP
            || per_files
                .get(&(fa.min(fb), fa.max(fb)))
                .copied()
                .unwrap_or(0)
                >= FILE_PAIR_CAP;
        if full {
            omitted += 1;
            continue;
        }
        *per_files.entry((fa.min(fb), fa.max(fb))).or_default() += 1;
        kept.push(pair);
    }
    (kept, omitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_sections_across_files_pair_up() {
        let stack = "The frontend uses React with Vite and Tailwind while the backend runs Hono on Node with Supabase for storage and auth";
        let texts = [
            Text {
                file: 0,
                text: stack,
            },
            Text {
                file: 1,
                text: &format!("{stack}. Deploys go to Fly.io in one region."),
            },
            Text {
                file: 1,
                text: "Completely different words about release steps, tags, and the changelog we keep for every version of the app",
            },
            Text {
                file: 0,
                text: stack,
            },
        ];
        let (pairs, omitted) = pairs(&texts);
        assert_eq!(omitted, 0);
        assert_eq!(pairs.len(), 2, "same-file copies are not paired: {pairs:?}");
        assert!(
            pairs
                .iter()
                .all(|(a, b, share)| texts[*a].file != texts[*b].file && *share == 1.0)
        );
    }
}
