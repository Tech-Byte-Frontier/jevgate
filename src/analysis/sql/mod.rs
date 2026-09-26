//! PostgreSQL statements for the access-control rule: a splitter that honors
//! comments, quotes and dollar quotes, and the few statement kinds whose final
//! state across migrations decides who reaches which rows. It locates and
//! scopes; it never decides a finding.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub start_line: usize,
    pub end_line: usize,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Policy {
        name: String,
        table: String,
    },
    DropPolicy {
        name: String,
        table: String,
    },
    Function {
        name: String,
        definer: bool,
    },
    DropFunction {
        name: String,
    },
    Table {
        name: String,
    },
    Grant {
        target: Option<String>,
    },
    /// A grant or revoke of EXECUTE on one function.
    Execute {
        function: String,
    },
    RowSecurity {
        table: String,
        enabled: bool,
    },
    Other,
}

mod split;
use split::quoted_end;
pub use split::statements;

/// Classifying reads only a statement's head: its kind, name and target.
const HEAD_TOKENS: usize = 48;

/// Lowercase words and punctuation, with quoted identifiers kept whole.
fn tokens(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = source.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c.is_whitespace() {
            continue;
        }
        if c == '"' {
            let end = quoted_end(source.as_bytes(), i);
            out.push(source[i + 1..end.saturating_sub(1).max(i + 1)].to_lowercase());
            while chars.peek().is_some_and(|(j, _)| *j < end) {
                chars.next();
            }
        } else if c.is_alphanumeric() || c == '_' {
            let mut word = c.to_lowercase().to_string();
            while let Some((_, d)) = chars.peek().copied() {
                if !(d.is_alphanumeric() || d == '_' || d == '$') {
                    break;
                }
                word.extend(d.to_lowercase());
                chars.next();
            }
            out.push(word);
        } else {
            out.push(c.to_string());
        }
        if out.len() > HEAD_TOKENS {
            break;
        }
    }
    out
}

/// A possibly schema-qualified name at `at`, and the index after it.
fn qualified(tokens: &[String], at: usize) -> Option<(String, usize)> {
    let first = tokens.get(at)?;
    if tokens.get(at + 1).is_some_and(|t| t == ".") {
        let second = tokens.get(at + 2)?;
        return Some((format!("{first}.{second}"), at + 3));
    }
    Some((first.clone(), at + 1))
}

/// The unqualified part of a name, which statements use interchangeably.
pub fn short(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// Tokens of a statement head and the matching helpers each kind uses.
struct Head(Vec<String>);

impl Head {
    fn is(&self, i: usize, word: &str) -> bool {
        self.0.get(i).is_some_and(|w| w == word)
    }

    /// The index after any of `words` starting at `i`.
    fn skip(&self, mut i: usize, words: &[&str]) -> usize {
        while words.iter().any(|w| self.is(i, w)) {
            i += 1;
        }
        i
    }

    /// Whether `words` appear in order starting at `i`.
    fn phrase(&self, i: usize, words: &[&str]) -> bool {
        words.iter().enumerate().all(|(k, w)| self.is(i + k, w))
    }

    fn name(&self, at: usize) -> Option<String> {
        qualified(&self.0, at).map(|(name, _)| name)
    }
}

pub fn classify(source: &str) -> Kind {
    let head = Head(tokens(source));
    let kind = match head.0.first().map(String::as_str) {
        Some("create") => create(&head, source),
        Some("drop") => dropped(&head),
        Some("grant") => Some(execute(&head).unwrap_or_else(|| grant(&head))),
        Some("revoke") => execute(&head),
        Some("alter") => row_security(&head),
        _ => None,
    };
    kind.unwrap_or(Kind::Other)
}

fn create(head: &Head, source: &str) -> Option<Kind> {
    if head.is(1, "policy") {
        return head.is(3, "on").then(|| {
            Some(Kind::Policy {
                name: head.0.get(2)?.clone(),
                table: head.name(4)?,
            })
        })?;
    }
    let at = head.skip(1, &["or", "replace"]);
    if head.is(at, "function") {
        return Some(Kind::Function {
            name: head.name(at + 1)?,
            definer: security_definer(source),
        });
    }
    let at = head.skip(1, &["unlogged", "temporary", "temp"]);
    head.is(at, "table").then(|| {
        Some(Kind::Table {
            name: head.name(head.skip(at + 1, &["if", "not", "exists"]))?,
        })
    })?
}

fn dropped(head: &Head) -> Option<Kind> {
    let at = head.skip(2, &["if", "exists"]);
    if head.is(1, "policy") && head.is(at + 1, "on") {
        return Some(Kind::DropPolicy {
            name: head.0.get(at)?.clone(),
            table: head.name(at + 2)?,
        });
    }
    head.is(1, "function").then(|| {
        Some(Kind::DropFunction {
            name: head.name(at)?,
        })
    })?
}

/// A grant or revoke on one function: who may call it.
fn execute(head: &Head) -> Option<Kind> {
    let on = head.0.iter().position(|w| w == "on")?;
    head.is(on + 1, "function").then(|| {
        Some(Kind::Execute {
            function: head.name(on + 2)?,
        })
    })?
}

/// Claims a statement reads from the token: `auth.jwt() ->> 'name'`, or a
/// path such as `-> 'app_metadata' ->> 'role'`.
pub fn jwt_claims(source: &str) -> Vec<String> {
    let text = source.to_lowercase();
    let mut claims: Vec<String> = Vec::new();
    for (at, _) in text.match_indices("jwt()") {
        let mut rest = text[at + "jwt()".len()..].trim_start();
        while let Some(after) = rest.strip_prefix("->>").or_else(|| rest.strip_prefix("->")) {
            let Some(quoted) = after.trim_start().strip_prefix('\'') else {
                break;
            };
            let Some(end) = quoted.find('\'') else {
                break;
            };
            let claim = quoted[..end].to_string();
            if !claims.contains(&claim) {
                claims.push(claim);
            }
            rest = quoted[end + 1..].trim_start();
        }
    }
    claims
}

/// A grant and the table it names, when it names one table.
fn grant(head: &Head) -> Kind {
    let target = head.0.iter().position(|w| w == "on").and_then(|on| {
        let at = head.skip(on + 1, &["table"]);
        let many = ["all", "schema", "function"].iter().any(|w| head.is(at, w));
        if many { None } else { head.name(at) }
    });
    Kind::Grant { target }
}

fn row_security(head: &Head) -> Option<Kind> {
    if !head.is(1, "table") {
        return None;
    }
    let (table, next) = qualified(&head.0, head.skip(2, &["only", "if", "exists"]))?;
    let enabled = head.is(next, "enable") || head.is(next, "force");
    let toggles = enabled || head.is(next, "disable");
    (toggles && head.phrase(next + 1, &["row", "level", "security"]))
        .then_some(Kind::RowSecurity { table, enabled })
}

fn security_definer(source: &str) -> bool {
    let words: Vec<String> = source
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    words
        .windows(2)
        .any(|w| w[0] == "security" && w[1] == "definer")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIGRATION: &str = "-- Profiles\ncreate table public.profiles (id uuid primary key, note text default 'a;b');\n\nalter table public.profiles enable row level security;\n/* policies */\ncreate policy \"Own profile\" on public.profiles\n  for select using (id = auth.uid());\ncreate or replace function public.touch() returns trigger\nlanguage plpgsql security definer as $$\nbegin\n  update public.profiles set note = 'x;y';\n  return new;\nend;\n$$;\ngrant select on table public.profiles to anon;\ndrop policy if exists \"Own profile\" on public.profiles;\n";

    #[test]
    fn statements_honor_quotes_dollar_bodies_and_comments() {
        let found = statements(MIGRATION);
        let kinds: Vec<Kind> = found.iter().map(|s| classify(&s.source)).collect();
        assert_eq!(
            kinds,
            [
                Kind::Table {
                    name: "public.profiles".into()
                },
                Kind::RowSecurity {
                    table: "public.profiles".into(),
                    enabled: true
                },
                Kind::Policy {
                    name: "own profile".into(),
                    table: "public.profiles".into()
                },
                Kind::Function {
                    name: "public.touch".into(),
                    definer: true
                },
                Kind::Grant {
                    target: Some("public.profiles".into())
                },
                Kind::DropPolicy {
                    name: "own profile".into(),
                    table: "public.profiles".into()
                },
            ]
        );
        assert_eq!((found[2].start_line, found[2].end_line), (6, 7));
        assert_eq!(
            classify("REVOKE EXECUTE ON FUNCTION public.hook FROM anon;"),
            Kind::Execute {
                function: "public.hook".into()
            }
        );
        assert_eq!(
            jwt_claims(
                "using (org = (select (auth.jwt() ->> 'organization_id')::uuid) and (auth.jwt() -> 'app_metadata' ->> 'role') = 'x')"
            ),
            ["organization_id", "app_metadata", "role"]
        );
        assert!(found[2].source.starts_with("create policy"));
        assert_eq!((found[3].start_line, found[3].end_line), (8, 14));
    }

    #[test]
    fn statements_step_over_characters_of_several_bytes() {
        // pgweb's booktown.sql holds U+FFFD outside quotes; a byte step panicked.
        let found = statements("insert into books values (1, \u{fffd}Dune\u{fffd});\nselect 1;\n");
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].source, "select 1;");
        let last = statements("select 1;\nselect 2 as caf\u{e9}");
        assert_eq!((last[1].start_line, last[1].end_line), (2, 2));
    }
}
