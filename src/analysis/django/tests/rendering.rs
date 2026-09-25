//! The templates a view renders and the lines that select a settings module.
use super::*;

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
