//! What a Python GraphQL schema's resolvers receive, sent beside the file's
//! path and language like the web framework roles: a graphene resolver's
//! arguments read as parameters any caller could pass, so DVGA's SSRF,
//! command and SQL injections through `resolve_*` and `mutate` arguments
//! were considers "if a caller passes outside input" instead of reviews.
use std::path::Path;

const RESOLVERS: &str = "GraphQL server code: its resolvers (graphene `resolve_*` methods and `mutate`, strawberry fields and mutations, ariadne `@query.field` and `@mutation.field` functions) receive the arguments of a client's query or mutation, so those arguments are client input, and `info.context` holds the request.";

/// The GraphQL facts of a Python file that imports a GraphQL server
/// library, or none.
pub(super) fn describe(path: &Path, source: &str) -> Option<&'static str> {
    if path.extension().and_then(|e| e.to_str()) != Some("py") {
        return None;
    }
    source
        .lines()
        .map(str::trim_start)
        .any(|line| {
            ["graphene", "strawberry", "ariadne"].iter().any(|library| {
                line.starts_with(&format!("import {library}"))
                    || line.starts_with(&format!("from {library}"))
            })
        })
        .then_some(RESOLVERS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_files_importing_a_graphql_server_library_have_resolvers() {
        let described = |path: &str, source: &str| describe(Path::new(path), source);
        assert_eq!(
            described(
                "core/views.py",
                "import graphene\n\nclass Query(graphene.ObjectType):\n    pass\n"
            ),
            Some(RESOLVERS)
        );
        assert!(
            described(
                "api/schema.py",
                "from strawberry.fastapi import GraphQLRouter\n"
            )
            .is_some()
        );
        assert!(
            described(
                "core/views.py",
                "import requests\n# uses graphene elsewhere\n"
            )
            .is_none()
        );
        assert!(described("schema.js", "import graphene\n").is_none());
    }
}
