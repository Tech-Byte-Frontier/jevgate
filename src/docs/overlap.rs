//! Candidate pairs of sections that may repeat each other across documents:
//! the share of the smaller section's three-word sequences found in the
//! other. The share only selects pairs to ask about; it never decides.
//! A section that pairs with two or more others heads a family: its members
//! are asked against it alone, not against each other, so a section repeated
//! in seven quickstarts is six questions, not twenty-one.
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

/// A section's three-word sequences, from its prose, commands and settings
/// but never its program code: documentation sites repeat the same API calls
/// in the code of unrelated pages, and pairing on code sent hundreds of pages
/// that shared a `streamText` call and nothing else, while the same setup
/// commands in two guides are what repeats. A section of code examples alone
/// is not paired.
fn shingles(text: &str) -> BTreeSet<String> {
    sequences(&outside_code(text))
}

/// Fence languages of program code, as opposed to commands, settings or
/// output.
const PROGRAM_LANGUAGES: &[&str] = &[
    "ts",
    "tsx",
    "typescript",
    "js",
    "jsx",
    "javascript",
    "mjs",
    "cjs",
    "py",
    "python",
    "python3",
    "pycon",
    "rs",
    "rust",
    "go",
    "java",
    "kotlin",
    "kt",
    "swift",
    "rb",
    "ruby",
    "php",
    "c",
    "cpp",
    "c++",
    "cs",
    "csharp",
    "vue",
    "svelte",
    "astro",
    "html",
    "css",
    "scss",
    "graphql",
    "dart",
    "scala",
    "elixir",
    "lua",
    "jinja",
    "html+jinja",
    "sql",
];

/// The text outside fenced code blocks and the blocks that are not program
/// code.
fn outside_code(text: &str) -> String {
    let mut fence: Option<(&str, bool)> = None;
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some((open, kept)) = fence {
            if trimmed.starts_with(open) {
                fence = None;
            } else if kept {
                out.push_str(line);
                out.push('\n');
            }
        } else if let Some(open) = ["```", "~~~"].into_iter().find(|f| trimmed.starts_with(f)) {
            let language = trimmed[open.len()..]
                .trim_start_matches(['`', '~', '{', '.'])
                .split(|c: char| c.is_whitespace() || c == '}' || c == ',')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            fence = Some((open, !PROGRAM_LANGUAGES.contains(&language.as_str())));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn sequences(text: &str) -> BTreeSet<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    words.windows(3).map(|w| w.join(" ")).collect()
}

/// A candidate pair: two indexes into the texts and their share.
type Pair = (usize, usize, f64);

/// Candidate pairs as indexes into `texts`, most shared first, with the
/// number omitted by the caps. Only sections of files `comparable` accepts
/// are paired.
pub fn pairs(texts: &[Text<'_>], comparable: &dyn Fn(usize, usize) -> bool) -> (Vec<Pair>, usize) {
    let sets: Vec<BTreeSet<String>> = texts.iter().map(|t| shingles(t.text)).collect();
    let mut found: Vec<Pair> = shared_counts(texts, &sets, comparable)
        .into_iter()
        .map(|((a, b), n)| (a, b, n as f64 / sets[a].len().min(sets[b].len()) as f64))
        .filter(|(_, _, share)| *share >= MIN_SHARE)
        .collect();
    found.sort_by(|x, y| y.2.total_cmp(&x.2).then((x.0, x.1).cmp(&(y.0, y.1))));
    capped(texts, families(found))
}

/// Pairs without those between two members of one family: each member is
/// asked against the family's head, the section with the most candidates.
fn families(found: Vec<Pair>) -> Vec<Pair> {
    let mut adjacent = BTreeMap::<usize, BTreeSet<usize>>::new();
    for &(a, b, _) in &found {
        adjacent.entry(a).or_default().insert(b);
        adjacent.entry(b).or_default().insert(a);
    }
    let mut nodes: Vec<(&usize, &BTreeSet<usize>)> = adjacent.iter().collect();
    nodes.sort_by(|x, y| y.1.len().cmp(&x.1.len()).then(x.0.cmp(y.0)));
    let mut head_of = BTreeMap::<usize, usize>::new();
    let mut heads = BTreeSet::new();
    for (&node, neighbors) in nodes {
        if head_of.contains_key(&node) || heads.contains(&node) {
            continue;
        }
        let free: Vec<usize> = neighbors
            .iter()
            .copied()
            .filter(|n| !head_of.contains_key(n) && !heads.contains(n))
            .collect();
        if free.len() >= 2 {
            heads.insert(node);
            head_of.extend(free.into_iter().map(|m| (m, node)));
        }
    }
    found
        .into_iter()
        .filter(|(a, b, _)| match (head_of.get(a), head_of.get(b)) {
            (Some(x), Some(y)) => x != y,
            _ => true,
        })
        .collect()
}

/// How many sequences each pair of sections from comparable files shares.
fn shared_counts(
    texts: &[Text<'_>],
    sets: &[BTreeSet<String>],
    comparable: &dyn Fn(usize, usize) -> bool,
) -> BTreeMap<(usize, usize), usize> {
    let mut index = BTreeMap::<&str, Vec<usize>>::new();
    for (i, set) in sets
        .iter()
        .enumerate()
        .filter(|(_, s)| s.len() >= MIN_SHINGLES)
    {
        for s in set {
            index.entry(s).or_default().push(i);
        }
    }
    let mut shared = BTreeMap::new();
    for owners in index.values() {
        for (n, &a) in owners.iter().enumerate() {
            for &b in owners[n + 1..].iter().filter(|&&b| {
                texts[a].file != texts[b].file && comparable(texts[a].file, texts[b].file)
            }) {
                *shared.entry((a, b)).or_default() += 1;
            }
        }
    }
    shared
}

/// The pairs kept under the run cap and the cap per pair of files.
fn capped(texts: &[Text<'_>], found: Vec<Pair>) -> (Vec<Pair>, usize) {
    let mut per_files = BTreeMap::<(usize, usize), usize>::new();
    let mut kept = Vec::new();
    let mut omitted = 0;
    for pair in found {
        let (fa, fb) = (texts[pair.0].file, texts[pair.1].file);
        let files = per_files.entry((fa.min(fb), fa.max(fb))).or_default();
        if kept.len() >= RUN_CAP || *files >= FILE_PAIR_CAP {
            omitted += 1;
            continue;
        }
        *files += 1;
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
        let (pairs, omitted) = pairs(&texts, &|_, _| true);
        assert_eq!(omitted, 0);
        assert_eq!(pairs.len(), 2, "same-file copies are not paired: {pairs:?}");
        assert!(
            pairs
                .iter()
                .all(|(a, b, share)| texts[*a].file != texts[*b].file && *share == 1.0)
        );
    }

    const PREREQUISITES: &str = "To follow this quickstart you will need Node.js 22 or later and pnpm installed on your local development machine and an API key";

    #[test]
    fn sections_pair_on_prose_and_commands_never_on_program_code() {
        let code = "```ts\nconst result = streamText({ model: gateway('gpt-4o'), prompt: 'Write a vegetarian lasagna recipe for four people', maxRetries: 2 });\n```\n";
        let a = format!(
            "Generating embeddings lets you compare the meaning of two texts and retrieve similar documents from a vector store you already run.\n{code}"
        );
        let b = format!(
            "Middleware wraps a language model so you can log calls, cache responses or add guardrails without changing the code that calls it.\n{code}"
        );
        let install = "Install:\n\n```bash\npnpm add ai @ai-sdk/react zod\npnpm add -D typescript tsx @types/node\nexport AI_GATEWAY_API_KEY=your-key-from-the-dashboard\npnpm dev --port 3000\n```\n";
        let texts = [
            Text { file: 0, text: &a },
            Text { file: 1, text: &b },
            Text {
                file: 2,
                text: code,
            },
            Text {
                file: 3,
                text: code,
            },
            Text {
                file: 4,
                text: install,
            },
            Text {
                file: 5,
                text: install,
            },
        ];
        let (pairs, _) = pairs(&texts, &|_, _| true);
        let found: Vec<(usize, usize)> = pairs.iter().map(|(a, b, _)| (*a, *b)).collect();
        assert_eq!(
            found,
            [(4, 5)],
            "code examples alone are not paired; commands are"
        );
    }

    #[test]
    fn a_family_is_asked_against_its_head_only() {
        let texts: Vec<Text<'_>> = (0..5)
            .map(|file| Text {
                file,
                text: PREREQUISITES,
            })
            .collect();
        let (pairs, omitted) = pairs(&texts, &|_, _| true);
        assert_eq!(omitted, 0);
        let found: Vec<(usize, usize)> = pairs.iter().map(|(a, b, _)| (*a, *b)).collect();
        assert_eq!(found, [(0, 1), (0, 2), (0, 3), (0, 4)]);
    }

    #[test]
    fn files_that_are_not_comparable_are_not_paired() {
        let texts: Vec<Text<'_>> = (0..3)
            .map(|file| Text {
                file,
                text: PREREQUISITES,
            })
            .collect();
        let (pairs, _) = pairs(&texts, &|a, b| a == 0 || b == 0);
        let found: Vec<(usize, usize)> = pairs.iter().map(|(a, b, _)| (*a, *b)).collect();
        assert_eq!(found, [(0, 1), (0, 2)]);
    }
}
