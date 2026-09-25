//! URL routes and management commands.
use super::*;

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
