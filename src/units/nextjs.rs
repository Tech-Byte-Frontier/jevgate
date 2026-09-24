//! What Next.js and React server directives make of a file, sent beside its
//! path and language: who calls its exports and where its code runs. A route
//! handler's `request` and a Server Action's arguments come from any client,
//! and a client component's requests leave from the user's browser; the
//! source alone does not say so, and the origin and URL checks otherwise
//! read them as parameters of unknown origin. Facts come from the path
//! within a package that depends on `next`, and from `'use server'` and
//! `'use client'` directives in any package.
use crate::packages::Package;
use std::path::{Component, Path};

const ROUTE_HANDLER: &str = "Next.js App Router route handler: its exported GET, POST, PUT, PATCH, DELETE, HEAD and OPTIONS functions answer HTTP requests from any client, so the request, its URL, search parameters, headers, cookies, body and the route `params` are client input, and what they return is the response; an error they throw becomes a generic 500 response.";
const API_ROUTE: &str = "Next.js Pages Router API route: its default export answers HTTP requests from any client, so `req.query`, `req.body`, headers and cookies are client input, and what it writes to `res` is the response.";
const SERVER_ACTIONS: &str = "Server Actions module ('use server'): every exported async function is an endpoint any client can call directly with arguments it chooses, including FormData fields. What it returns goes back to that client as is; in production, Next.js replaces the message of an error it throws with a generic one and a digest.";
const INLINE_ACTIONS: &str = "Functions marked 'use server' inside it are Server Actions: any client can call them with arguments it chooses, including FormData fields.";
const CLIENT: &str = "Client component module ('use client'): it runs in the user's browser, so the requests it sends reach only what that user can, and what it renders is shown to that user.";
const ERROR_BOUNDARY: &str = "Next.js error boundary: a component that runs in the browser of the user whose page failed and shows that user the error it receives. It sends no response: in production, Next.js has already replaced the message of an error thrown on the server with a generic one and a digest, and an error thrown in the browser is that user's own.";
const PAGE: &str = "Next.js App Router page, layout or template: a server component unless marked 'use client'; the route `params` and `searchParams` it receives are client input, and what it renders is sent to the browser.";
const PAGES_PAGE: &str = "Next.js Pages Router page: `getServerSideProps` and `getStaticProps` run on the server, where `context.query`, `context.params` and `context.req` are client input; the component renders in the browser.";
const MIDDLEWARE: &str = "Next.js middleware: it runs on the server before every request its `matcher` selects, so the request, its URL, headers and cookies are client input, and a redirect or rewrite it returns is the response.";
const CONFIG: &str = "Next.js configuration: `headers()` sets headers on responses, `redirects()` and `rewrites()` route requests, `images.remotePatterns` chooses the hosts the image optimizer fetches, and values under `env` are inlined into browser code.";
const SEGMENT_SETTINGS: &str = "Its exported route settings (`config`, `runtime`, `revalidate`, `dynamic`, `maxDuration`, `metadata`) must be literals, since Next.js reads them at build time.";

/// The Next.js facts of one file, joined, or none.
pub(super) fn describe(path: &Path, source: &str, package: Option<&Package>) -> Option<String> {
    let facts = facts(path, source, package);
    (!facts.is_empty()).then(|| facts.join(" "))
}

fn facts(path: &Path, source: &str, package: Option<&Package>) -> Vec<&'static str> {
    if !script(path) {
        return Vec::new();
    }
    let directive = directive(source);
    let next = package.filter(|p| p.dependencies.contains("next"));
    let within = next.map(|p| path.strip_prefix(&p.dir).unwrap_or(path));
    let role = within.and_then(|within| role(within, directive));
    let mut facts = Vec::new();
    match directive {
        Some(Directive::Server) => facts.push(SERVER_ACTIONS),
        Some(Directive::Client) if role != Some(ERROR_BOUNDARY) => facts.push(CLIENT),
        _ => {}
    }
    if let Some(role) = role {
        facts.push(role);
        if [ROUTE_HANDLER, PAGE, MIDDLEWARE].contains(&role) {
            facts.push(SEGMENT_SETTINGS);
        }
    }
    if directive != Some(Directive::Server) && inline_actions(source) {
        facts.push(INLINE_ACTIONS);
    }
    facts
}

/// JavaScript and TypeScript sources, where the directives and file
/// conventions apply.
fn script(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"].contains(&e))
}

/// The role a file's path gives it in a Next.js package, from the path
/// within the package; `src/` holds the same conventions as the root.
fn role(within: &Path, directive: Option<Directive>) -> Option<&'static str> {
    let parts: Vec<&str> = within
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    let parts = match parts.as_slice() {
        ["src", rest @ ..] if !rest.is_empty() => rest,
        all => all,
    };
    let (file, directories) = parts.split_last()?;
    let stem = file.split('.').next().unwrap_or("");
    if directories.is_empty() {
        return match stem {
            "middleware" | "proxy" => Some(MIDDLEWARE),
            "next" if file.starts_with("next.config.") => Some(CONFIG),
            _ => None,
        };
    }
    match directories[0] {
        "app" => match stem {
            "route" => Some(ROUTE_HANDLER),
            "error" | "global-error" => Some(ERROR_BOUNDARY),
            "page" | "layout" | "template" | "default" if directive.is_none() => Some(PAGE),
            _ => None,
        },
        "pages" if directories.get(1) == Some(&"api") => Some(API_ROUTE),
        "pages" if !stem.starts_with('_') => Some(PAGES_PAGE),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Directive {
    Server,
    Client,
}

/// The directive a module starts with, after comments and blank lines.
fn directive(source: &str) -> Option<Directive> {
    let mut rest = source.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        if let Some(line) = rest.strip_prefix("//") {
            rest = line.split_once('\n').map_or("", |(_, next)| next);
        } else if let Some(block) = rest.strip_prefix("/*") {
            rest = block.split_once("*/").map_or("", |(_, next)| next);
        } else {
            break;
        }
    }
    let quoted = |text: &str| {
        ["'", "\""]
            .iter()
            .any(|q| rest.starts_with(&format!("{q}{text}{q}")))
    };
    if quoted("use server") {
        Some(Directive::Server)
    } else if quoted("use client") {
        Some(Directive::Client)
    } else {
        None
    }
}

/// Whether a function body inside the file starts with `'use server'`.
fn inline_actions(source: &str) -> bool {
    ["'use server'", "\"use server\""].iter().any(|needle| {
        source
            .match_indices(needle)
            .any(|(at, _)| source[..at].trim_end().ends_with(['{', ';']))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn next_package(dir: &str) -> Package {
        Package {
            dir: PathBuf::from(dir),
            name: Some("web".into()),
            dependencies: ["next".to_string(), "react".to_string()].into(),
        }
    }

    fn described(path: &str, source: &str, package: Option<&Package>) -> String {
        describe(Path::new(path), source, package).unwrap_or_default()
    }

    #[test]
    fn file_conventions_name_the_role_within_a_next_package() {
        let package = next_package("apps/web");
        let with = |path: &str, source: &str| described(path, source, Some(&package));
        assert!(with("apps/web/app/api/users/route.ts", "").starts_with(ROUTE_HANDLER));
        assert!(with("apps/web/src/app/(shop)/[id]/route.js", "").starts_with(ROUTE_HANDLER));
        assert!(with("apps/web/pages/api/export.ts", "").starts_with(API_ROUTE));
        assert!(with("apps/web/src/pages/index.tsx", "").starts_with(PAGES_PAGE));
        assert!(with("apps/web/pages/_app.tsx", "").is_empty());
        assert!(with("apps/web/middleware.ts", "").starts_with(MIDDLEWARE));
        assert!(with("apps/web/src/proxy.ts", "").starts_with(MIDDLEWARE));
        assert!(with("apps/web/next.config.mjs", "").starts_with(CONFIG));
        assert!(with("apps/web/app/blog/page.tsx", "").starts_with(PAGE));
        assert!(with("apps/web/app/blog/page.tsx", "").contains(SEGMENT_SETTINGS));
        assert_eq!(
            with("apps/web/app/blog/error.tsx", "'use client';\n"),
            ERROR_BOUNDARY
        );
        assert_eq!(with("apps/web/app/global-error.tsx", ""), ERROR_BOUNDARY);
        assert!(with("apps/web/lib/db.ts", "").is_empty());
        assert!(with("apps/web/app/blog/styles.css", "").is_empty());
        // Outside a package that depends on `next`, paths mean nothing.
        let other = Package {
            dependencies: ["express".to_string()].into(),
            ..next_package("")
        };
        assert!(described("app/api/route.ts", "", Some(&other)).is_empty());
        assert!(described("middleware.ts", "", None).is_empty());
    }

    #[test]
    fn directives_mark_server_actions_and_client_components_in_any_package() {
        let actions = "// Actions\n/* for forms */\n'use server';\n\nexport async function save(formData: FormData) {}\n";
        assert_eq!(described("lib/actions.ts", actions, None), SERVER_ACTIONS);
        assert_eq!(
            described(
                "components/search.tsx",
                "\"use client\"\nexport function A() {}\n",
                None
            ),
            CLIENT
        );
        let package = next_package("");
        let page = "'use client'\nexport default function Page() {}\n";
        assert_eq!(described("app/page.tsx", page, Some(&package)), CLIENT);
        let inline = "export default function Page() {\n  async function create(formData: FormData) {\n    'use server';\n    await save(formData);\n  }\n  return <form action={create} />;\n}\n";
        let found = described("app/new/page.tsx", inline, Some(&package));
        assert!(
            found.starts_with(PAGE) && found.ends_with(INLINE_ACTIONS),
            "{found}"
        );
        assert!(described("lib/text.ts", "const hint = \"use server\";\n", None).is_empty());
    }
}
