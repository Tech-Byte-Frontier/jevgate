//! What SvelteKit's file conventions make of a file, sent beside its path
//! and language like the Next.js roles: who calls its exports and what the
//! framework does with them. A form action's `request` comes from any
//! client, and `cookies.set` has secure defaults; the source alone does not
//! say so, and cookies set without options read as missing HttpOnly and
//! Secure. Facts come from the path within a package that depends on
//! `@sveltejs/kit`.
use crate::packages::Package;
use std::path::{Component, Path};

const COOKIES: &str = "`cookies.set` makes a cookie httpOnly, secure (except on localhost) and sameSite=lax unless its options turn them off. `redirect(status, location)` and `error(status, body)` take an HTTP status code.";
const SERVER_LOAD: &str = "SvelteKit server load and form actions: `load` and each function of `actions` run on the server for requests from any client, so `request`, its form data, `url`, `params` and `cookies` are client input, and what `load` returns is sent to the page.";
const ENDPOINT: &str = "SvelteKit endpoint: its exported GET, POST, PUT, PATCH, DELETE, OPTIONS and fallback functions answer HTTP requests from any client, so `request`, `url`, `params` and `cookies` are client input, and the Response they return is the response.";
const HOOKS: &str = "SvelteKit server hooks: `handle` runs on the server before every request, so `event.request`, `event.url` and `event.cookies` are client input; for an unexpected error, clients receive only what `handleError` returns.";

/// The SvelteKit facts of one file, joined, or none.
pub(super) fn describe(path: &Path, package: Option<&Package>) -> Option<String> {
    let kit = package.filter(|p| p.dependencies.contains("@sveltejs/kit"))?;
    let within = path.strip_prefix(&kit.dir).unwrap_or(path);
    let role = role(within)?;
    Some(format!("{role} {COOKIES}"))
}

/// The role a file's path gives it in a SvelteKit package: server files of
/// `src/routes` and the server hooks.
fn role(within: &Path) -> Option<&'static str> {
    let parts: Vec<&str> = within
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    let (file, directories) = parts.split_last()?;
    let (stem, extension) = file.rsplit_once('.')?;
    if !["js", "ts"].contains(&extension) || directories.first() != Some(&"src") {
        return None;
    }
    match (stem, directories.get(1)) {
        ("hooks.server", None) => Some(HOOKS),
        ("+page.server" | "+layout.server", Some(&"routes")) => Some(SERVER_LOAD),
        ("+server", Some(&"routes")) => Some(ENDPOINT),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn kit() -> Package {
        Package {
            dir: PathBuf::from("web"),
            name: None,
            dependencies: ["@sveltejs/kit".to_string()].into(),
        }
    }

    #[test]
    fn server_routes_endpoints_and_hooks_have_roles_in_a_sveltekit_package() {
        let kit = kit();
        let described = |path: &str| describe(Path::new(path), Some(&kit));
        for (path, role) in [
            ("web/src/routes/login/+page.server.js", SERVER_LOAD),
            ("web/src/routes/+layout.server.ts", SERVER_LOAD),
            ("web/src/routes/api/items/+server.ts", ENDPOINT),
            ("web/src/hooks.server.js", HOOKS),
        ] {
            let text = described(path).unwrap();
            assert!(
                text.starts_with(role) && text.contains("httpOnly"),
                "{path}"
            );
        }
        for path in [
            "web/src/routes/login/+page.svelte",
            "web/src/routes/login/+page.js",
            "web/src/lib/api.js",
            "web/src/hooks.client.js",
            "web/routes/+server.ts",
        ] {
            assert_eq!(described(path), None, "{path}");
        }
        let plain = Package {
            dependencies: ["vite".to_string()].into(),
            ..kit
        };
        assert_eq!(
            describe(Path::new("web/src/routes/+server.ts"), Some(&plain)),
            None
        );
    }
}
