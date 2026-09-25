//! Django code; its routes and management commands are in `routes`, the
//! templates it renders and the lines selecting settings in `rendering`, and
//! settings modules and their secrets in `settings`.
mod rendering;
mod routes;
mod settings;

use super::*;

pub(super) fn tree(source: &str) -> tree_sitter::Tree {
    crate::syntax::parse(Path::new("a.py"), source)
        .unwrap()
        .unwrap()
}

#[test]
fn django_code_is_python_that_imports_django_or_rest_framework() {
    let check = |path: &str, source: &str| {
        imports_django(Path::new(path), tree(source).root_node(), source)
    };
    assert!(check(
        "app/views.py",
        "from django.shortcuts import render\n"
    ));
    assert!(check(
        "app/api.py",
        "import rest_framework.views as views\n"
    ));
    assert!(check(
        "app/compat.py",
        "try:\n    from django.urls import path\nexcept ImportError:\n    path = None\n"
    ));
    assert!(!check(
        "app/main.py",
        "from fastapi import FastAPI\nimport djangoish\n"
    ));
    assert!(!check(
        "app/views.ts",
        "from django.shortcuts import render\n"
    ));
}
