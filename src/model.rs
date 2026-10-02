//! What a model name says: a pinned version, whose answers never change, or an
//! alias that can move to a new version; and what its input costs.

/// The longest model name accepted from a provider.
const MAX_NAME_BYTES: usize = 128;

/// Jev 1.13's published price in dollars per million input tokens; output
/// tokens are free. TypeSafe's models page (https://docs.typesafe.ai/models),
/// checked on `PRICE_CHECKED`; OpenRouter's and Vercel AI Gateway's listings
/// give the same price.
pub const INPUT_USD_PER_MILLION: f64 = 0.042;
pub const PRICE_CHECKED: &str = "2026-09-28";

/// Model lines with a published price. A name is priced when its base name is
/// the line, one of its versions or a dated snapshot of it: `jev-1.13`,
/// `jev-1.13.0`, `typesafe/jev-1.13`, and `typesafe/jev-1.13-20260917`, the
/// endpoint OpenRouter lists for `typesafe/jev-1.13` at the same price
/// (checked on `PRICE_CHECKED`). An alias that names no version, such as
/// `typesafe-ai/jev`, is not: its price follows whatever it points to.
const PRICED_LINES: [&str; 1] = ["jev-1.13"];

/// Digits in a snapshot's date, as in `-20260917`.
const SNAPSHOT_DATE_DIGITS: usize = 8;

/// A name a provider may return: letters, digits and `-_.`, with `/` between a
/// gateway's namespace and the model (`typesafe/jev-1.13`) and `~` for
/// OpenRouter's moving aliases (`~typesafe/jev-latest`).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.~/".contains(&c))
}

/// The name without a gateway's namespace: `jev-1.13` of `typesafe/jev-1.13`.
pub fn base_name(name: &str) -> &str {
    name.rsplit_once('/').map_or(name, |(_, base)| base)
}

/// A pinned version: the base name ends in an `x.y.z` version, as in
/// `jev-1.13.0` or `typesafe/jev-1.13.0`, and no `~` marks it as moving. Every
/// other name is an alias that can move to a new version: `jev`, `jev-1.13`
/// and `jev-latest` in TypeSafe's docs, and the gateways' `typesafe-ai/jev` and
/// `~typesafe/jev-latest`.
pub fn pinned(name: &str) -> bool {
    let version = base_name(name).rsplit('-').next().unwrap_or_default();
    !name.contains('~') && dotted_numbers(version, 3)
}

/// Estimated dollars for `tokens` input tokens answered by `name`; none when
/// its price is unknown. No tokens cost nothing, whatever the model.
pub fn usd(name: &str, tokens: u64) -> Option<f64> {
    (tokens == 0 || priced(name)).then(|| tokens as f64 * INPUT_USD_PER_MILLION / 1_000_000.0)
}

/// Whether `name` is one of the `PRICED_LINES`, a version of one or a
/// dated snapshot of one.
fn priced(name: &str) -> bool {
    let base = base_name(name);
    PRICED_LINES.iter().any(|line| {
        base.strip_prefix(line).is_some_and(|rest| {
            rest.is_empty()
                || rest
                    .strip_prefix('.')
                    .is_some_and(|patch| dotted_numbers(patch, 1))
                || rest.strip_prefix('-').is_some_and(|date| {
                    date.len() == SNAPSHOT_DATE_DIGITS && date.bytes().all(|c| c.is_ascii_digit())
                })
        })
    })
}

/// Whether `text` is `count` runs of digits joined by dots: `1.13.0` for three.
fn dotted_numbers(text: &str, count: usize) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == count
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_names_ending_in_an_x_y_z_version_are_pinned() {
        for name in ["jev-1.13.0", "typesafe/jev-1.13.0", "typesafe-ai/jev-2.0.1"] {
            assert!(pinned(name), "{name}");
        }
        for name in [
            "jev",
            "jev-1.13",
            "jev-latest",
            "jev-preview",
            "typesafe/jev-1.13",
            "typesafe-ai/jev",
            "~typesafe/jev-latest",
            "~typesafe/jev-1.13.0",
            "jev-1.13.0-rc1",
            "jev-1..0",
            "other-version",
        ] {
            assert!(!pinned(name), "{name}");
        }
    }

    #[test]
    fn gateway_names_are_valid_and_their_base_is_the_model() {
        for name in [
            "jev-1.13.0",
            "typesafe/jev-1.13",
            "~typesafe/jev-latest",
            "typesafe-ai/jev",
        ] {
            assert!(valid_name(name), "{name}");
        }
        for name in [
            "",
            "jev 1",
            "jev\n",
            "jev@1",
            &"j".repeat(MAX_NAME_BYTES + 1),
        ] {
            assert!(!valid_name(name), "{name:?}");
        }
        assert_eq!(base_name("typesafe/jev-1.13"), "jev-1.13");
        assert_eq!(base_name("jev-1.13.0"), "jev-1.13.0");
    }

    #[test]
    fn the_jev_1_13_line_is_priced_under_any_namespace_and_aliases_are_not() {
        for name in [
            "jev-1.13.0",
            "jev-1.13",
            "typesafe/jev-1.13",
            "typesafe-ai/jev-1.13.2",
            // OpenRouter's endpoint for typesafe/jev-1.13, at $0.000000042 a token.
            "typesafe/jev-1.13-20260917",
        ] {
            let usd = usd(name, 1_000_000).unwrap_or_else(|| panic!("{name}"));
            assert!((usd - INPUT_USD_PER_MILLION).abs() < 1e-12, "{name}");
        }
        for name in [
            "jev-latest",
            "typesafe-ai/jev",
            "~typesafe/jev-latest",
            "jev-1.130",
            "jev-1.13.x",
            "jev-1.13-2026091",
            "jev-1.13-rc1",
        ] {
            assert_eq!(usd(name, 1_000), None, "{name}");
        }
        assert_eq!(usd("typesafe-ai/jev", 0), Some(0.0));
    }
}
