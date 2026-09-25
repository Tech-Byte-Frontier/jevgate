//! Django code, its routes, management commands, templates and settings
//! selections; settings modules and their secrets are in `settings`.
mod settings;

use super::*;

fn tree(source: &str) -> tree_sitter::Tree {
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

#[test]
fn urlconf_routes_name_their_views_and_keep_the_pattern() {
    let source = "urlpatterns = [\n    path('orders/<int:order_id>/', views.order_detail, name='detail'),\n    re_path(r'^t/(?P<task_id>\\d+)/$',\n        views.task_edit),\n    path('edit/', OrderEditView.as_view(), name='edit'),\n    url(r'^$', 'shop.views.index'),\n    path('api/', include(router.urls)),\n]\nrouter.register(r'orders', api.OrderViewSet, basename='order')\n";
    let found = routes(tree(source).root_node(), source);
    let views: Vec<(Option<&str>, &str, usize)> = found
        .iter()
        .map(|r| (r.module.as_deref(), r.view.as_str(), r.line))
        .collect();
    assert_eq!(
        views,
        [
            (Some("views"), "order_detail", 2),
            (Some("views"), "task_edit", 3),
            (None, "OrderEditView", 5),
            (Some("views"), "index", 6),
            (Some("api"), "OrderViewSet", 9),
        ],
        "an include routes to no view"
    );
    assert_eq!(
        found[1].text, "re_path(r'^t/(?P<task_id>\\d+)/$', views.task_edit)",
        "a route spread over lines is shown on one"
    );
    let views_py = Path::new("shop/views.py");
    assert!(routes_to(&found[0], views_py, "order_detail", ""));
    assert!(!routes_to(
        &found[0],
        Path::new("shop/api.py"),
        "order_detail",
        ""
    ));
    assert!(routes_to(&found[2], views_py, "post", "OrderEditView"));
    assert!(!routes_to(
        &found[2],
        views_py,
        "clean_note",
        "OrderEditView"
    ));
    assert!(routes_to(
        &found[4],
        Path::new("shop/api/__init__.py"),
        "list",
        "OrderViewSet"
    ));
}

#[test]
fn management_commands_are_named_by_their_module() {
    let command = |path: &'static str| management_command(Path::new(path));
    assert_eq!(
        command("shop/orders/management/commands/export_orders.py"),
        Some("export_orders")
    );
    assert_eq!(command("shop/orders/management/commands/__init__.py"), None);
    assert_eq!(command("shop/orders/commands/export_orders.py"), None);
}

#[test]
fn templates_are_named_after_their_templates_directory_and_list_unescaped_lines() {
    assert_eq!(
        template_name(Path::new("shop/orders/templates/orders/search.html")).as_deref(),
        Some("orders/search.html")
    );
    assert_eq!(template_name(Path::new("shop/templates.html")), None);
    assert_eq!(
        unescaped_lines(
            "<h1>{{ term }}</h1>\n<p>{{ term | safe }}</p>\n{% autoescape off %}{{ x }}{% endautoescape %}\n"
        ),
        [
            "2: <p>{{ term | safe }}</p>",
            "3: {% autoescape off %}{{ x }}{% endautoescape %}"
        ]
    );
    let templates = [Template {
        name: "orders/search.html".into(),
        path: "shop/orders/templates/orders/search.html".into(),
        unescaped: Vec::new(),
    }];
    assert_eq!(
        rendered(
            "return render(request, 'orders/search.html', {})",
            &templates
        )
        .len(),
        1
    );
    assert!(
        rendered(
            "return render(request, 'orders/search_safe.html', {})",
            &templates
        )
        .is_empty()
    );
}

#[test]
fn selections_name_a_settings_module_by_its_dotted_path() {
    let dockerfile = selections_in(
        Path::new("Dockerfile"),
        "FROM python\nENV DJANGO_SETTINGS_MODULE=shop.settings.production\n",
    );
    let manage = selections_in(
        Path::new("manage.py"),
        "os.environ.setdefault('DJANGO_SETTINGS_MODULE', 'shop.settings.dev')\n",
    );
    let all: Vec<Selection> = dockerfile.into_iter().chain(manage).collect();
    let production = selected_by(Path::new("shop/settings/production.py"), &all);
    assert_eq!(production.len(), 1);
    assert_eq!(production[0].line, 2);
    assert!(selected_by(Path::new("shop/settings/prod.py"), &all).is_empty());
    assert!(!selection_file(Path::new(".env.production")));
    assert!(selection_file(Path::new("deploy/app.yaml")));
}
