//! What a model name says: a pinned version, whose answers never change, or an
//! alias that can move to a new version.

/// The longest model name accepted from a provider.
const MAX_NAME_BYTES: usize = 128;

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
    let parts: Vec<&str> = version.split('.').collect();
    !name.contains('~')
        && parts.len() == 3
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
}
