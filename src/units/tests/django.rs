//! Django: views and settings modules asked the Django checks with their
//! routes, templates, constants and the settings modules that import them;
//! weak settings named by a check; error views and middleware as handlers.
use super::*;

const VIEWS: &str = "import pickle\n\nfrom django.shortcuts import redirect, render\n\nINVOICE_DIR = '/srv/invoices'\n\n\ndef search(request):\n    term = request.GET.get('q', '')\n    rows = Order.objects.raw(\"SELECT * FROM orders WHERE note LIKE '%\" + term + \"%'\")\n    return render(request, 'orders/search.html', {'rows': rows, 'term': term})\n\n\ndef invoice(request):\n    name = request.GET['file']\n    return FileResponse(open(INVOICE_DIR + '/' + name, 'rb'))\n\n\ndef import_cart(request):\n    cart = pickle.loads(request.body)\n    return redirect(request.GET['next'])\n";

const URLS: &str = "from django.urls import path\n\nfrom shop import views\n\nurlpatterns = [\n    path('search/', views.search, name='search'),\n    path('invoice/', views.invoice),\n]\n\nhandler500 = 'shop.views.server_error'\n";

const PLAIN: &str = "import subprocess\n\n\ndef archive(name):\n    subprocess.run('tar czf ' + name + '.tgz ' + name, shell=True)\n    return open(name + '.tgz', 'rb').read()\n";

fn django_project() -> (Project, CheckArgs) {
    project_with(
        &[
            ("shop/views.py", VIEWS),
            ("shop/urls.py", URLS),
            ("tools/archive.py", PLAIN),
            (
                "shop/templates/orders/search.html",
                "<h1>Results for {{ term|safe }}</h1>\n",
            ),
        ],
        &catalog::SECURITY,
    )
}

/// The trace request of the injection unit of the function `name`.
fn injection_trace<'p>(plan: &'p Plan, name: &str) -> &'p Value {
    plan.files
        .values()
        .flat_map(|f| &f.units)
        .find_map(|u| match &u.detail {
            Detail::Security {
                trace: Some((request, _)),
                ..
            } if u.rule == catalog::INJECTION && u.name == name => Some(request),
            _ => None,
        })
        .unwrap_or_else(|| panic!("an injection trace for {name}"))
}

fn asked(request: &Value) -> Vec<String> {
    request["questions"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

#[test]
fn django_views_are_asked_the_django_checks_and_other_python_the_common_ones() {
    let (project, options) = django_project();
    let (_, plan) = planned(&project, &options);
    let view = injection_trace(&plan, "search");
    let questions = asked(view);
    for check in ["redirect", "deserialize", "sql", "markup"] {
        assert!(questions.contains(&check.to_string()), "{questions:?}");
    }
    assert!(
        view["questions"]["sql"]["criteria"]["true"]
            .as_str()
            .unwrap()
            .contains("RawSQL")
    );
    let plain = injection_trace(&plan, "archive");
    let questions = asked(plain);
    assert!(
        !questions.contains(&"deserialize".to_string()),
        "{questions:?}"
    );
    assert_eq!(
        plain["questions"]["redirect"],
        questions::UNHANDLED[6].body("function.source")
    );
    assert!(
        view["questions"]["redirect"]["criteria"]["false"]
            .as_str()
            .unwrap()
            .contains("reverse()")
    );
    assert_eq!(
        plain["questions"]["sql"],
        questions::UNHANDLED[0].body("function.source"),
        "code outside Django keeps the common checks as they were"
    );
    // The presence questions differ the same way.
    let first = |file: &str| {
        plan.requests
            .iter()
            .find(|p| {
                p.request["jevgate"]["stage"] == "security"
                    && p.request["state"]["file"]["path"] == file
            })
            .unwrap()
            .request["questions"]["f0_interpreted"]["instructions"]["question"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(first("shop/views.py").contains("deserializer"));
    assert!(!first("tools/archive.py").contains("deserializer"));
}

#[test]
fn a_view_is_sent_with_its_routes_unescaped_templates_and_module_constants() {
    let (project, options) = django_project();
    let (_, plan) = planned(&project, &options);
    let search = &injection_trace(&plan, "search")["state"]["function"];
    assert_eq!(
        search["url_routes_that_send_requests_to_it"],
        json!(["shop/urls.py:6: path('search/', views.search, name='search')"])
    );
    assert_eq!(
        search["templates_it_renders_that_write_values_without_escaping"],
        json!([{
            "template": "shop/templates/orders/search.html",
            "unescaped_output": ["1: <h1>Results for {{ term|safe }}</h1>"],
        }])
    );
    let invoice = &injection_trace(&plan, "invoice")["state"]["function"];
    assert_eq!(
        invoice["module_constants_it_uses"],
        json!(["INVOICE_DIR = '/srv/invoices'"])
    );
    assert!(invoice["templates_it_renders_that_write_values_without_escaping"].is_null());
    let cart = &injection_trace(&plan, "import_cart")["state"]["function"];
    assert!(
        cart["url_routes_that_send_requests_to_it"].is_null(),
        "no route names it"
    );
}

#[test]
fn a_django_management_command_is_marked_as_run_by_hand() {
    let project = Project::new();
    project.write(
        "shop/management/commands/export_orders.py",
        "from django.core.management.base import BaseCommand\n\n\nclass Command(BaseCommand):\n    def handle(self, *args, **options):\n        with open(options['out'], 'w') as out:\n            out.write(dump(options['status']))\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let (_, plan) = planned(&project, &options);
    let state = &injection_trace(&plan, "Command::handle")["state"]["function"];
    assert!(
        state["django_management_command"]
            .as_str()
            .unwrap()
            .contains("`manage.py export_orders`")
    );
}

/// A settings project: shared settings, a development module the manage
/// script selects, and a production module the container selects.
fn settings_project() -> (Project, CheckArgs) {
    project_with(
        &[
            (
                "site/settings/base.py",
                "import os\n\nSECRET_KEY = 'dev-key-1234'\nDEBUG = True\nINSTALLED_APPS = ['shop']\nCORS_ALLOW_ALL_ORIGINS = True\n",
            ),
            (
                "site/settings/dev.py",
                "from .base import *  # noqa\n\nDEBUG = True\nALLOWED_HOSTS = ['*']\n",
            ),
            (
                "site/settings/production.py",
                "import os\n\nfrom .base import *  # noqa\n\nDEBUG = False\nSECRET_KEY = os.environ['DJANGO_SECRET_KEY']\n",
            ),
            (
                "manage.py",
                "import os\n\nos.environ.setdefault('DJANGO_SETTINGS_MODULE', 'site.settings.dev')\n",
            ),
            (
                "Dockerfile",
                "FROM python:3.12\nENV DJANGO_SETTINGS_MODULE=site.settings.production\n",
            ),
        ],
        &[catalog::UNSAFE_SETTINGS],
    )
}

fn module_request<'p>(plan: &'p Plan, path: &str) -> &'p Value {
    &plan
        .requests
        .iter()
        .find(|p| {
            p.request["state"]["file"]["path"] == path && p.request["state"]["module"].is_object()
        })
        .unwrap_or_else(|| panic!("a module request for {path}"))
        .request
}

#[test]
fn a_settings_module_is_sent_redacted_with_the_modules_that_import_it_and_what_selects_them() {
    let (project, options) = settings_project();
    let (_, plan) = planned(&project, &options);
    let base = module_request(&plan, "site/settings/base.py");
    assert!(!base.to_string().contains("dev-key-1234"), "{base}");
    let module = &base["state"]["module"];
    assert!(
        module["source"]
            .as_str()
            .unwrap()
            .contains("SECRET_KEY = '<redacted 12-character literal>'")
    );
    assert_eq!(
        module["settings_modules_that_import_it"],
        json!([
            {
                "module": "site/settings/dev.py",
                "selected_as_the_settings_to_run_with_by": [
                    "manage.py:3: os.environ.setdefault('DJANGO_SETTINGS_MODULE', 'site.settings.dev') (only a default: a DJANGO_SETTINGS_MODULE set in the environment replaces it)"
                ],
                "sets_these_settings_again": ["DEBUG = True"],
            },
            {
                "module": "site/settings/production.py",
                "selected_as_the_settings_to_run_with_by": [
                    "Dockerfile:2: ENV DJANGO_SETTINGS_MODULE=site.settings.production"
                ],
                "sets_these_settings_again": [
                    "DEBUG = False",
                    "SECRET_KEY = os.environ['DJANGO_SECRET_KEY']"
                ],
            },
        ])
    );
    let production = &module_request(&plan, "site/settings/production.py")["state"]["module"];
    assert_eq!(
        production["selected_as_the_settings_to_run_with_by"],
        json!(["Dockerfile:2: ENV DJANGO_SETTINGS_MODULE=site.settings.production"])
    );
    assert!(production["settings_modules_that_import_it"].is_null());
    let questions = asked(module_request(&plan, "site/settings/dev.py"));
    assert_eq!(questions, ["m0_weakened"]);
}

#[test]
fn a_weak_django_setting_must_be_named_by_a_check_and_development_settings_are_lower() {
    let (project, mut options) = settings_project();
    let finding = |nouls: &[(&'static str, f64)], options: &CheckArgs, path: &str| {
        run_with_nouls(&project, options, nouls)
            .files
            .into_iter()
            .find(|f| f.path.to_str() == Some(path))
            .unwrap()
            .findings
            .into_iter()
            .next()
    };
    let unnamed = finding(&[("weakened", 0.95)], &options, "site/settings/base.py").unwrap();
    assert_eq!(unnamed.strength, Strength::Note, "presence alone");
    assert!(
        unnamed
            .message
            .contains("no specific check named the setting"),
        "{}",
        unnamed.message
    );
    options.refresh = true;
    let named = finding(
        &[("weakened", 0.95), ("debug", 0.95), ("literal_secret", 0.9)],
        &options,
        "site/settings/base.py",
    )
    .unwrap();
    assert_eq!(named.strength, Strength::Review);
    assert_eq!(named.category.as_deref(), Some("CWE-489 active debug code"));
    assert!(
        named
            .message
            .starts_with("Settings module shows detailed error pages")
    );
    assert!(
        named
            .message
            .contains("Also found: CWE-798 hard-coded credentials."),
        "{}",
        named.message
    );
    options.refresh = true;
    let development = finding(
        &[("weakened", 0.95), ("debug", 0.95), ("dev_only", 0.95)],
        &options,
        "site/settings/dev.py",
    )
    .unwrap();
    assert_eq!(development.strength, Strength::Note, "two levels lower");
    assert!(
        development
            .message
            .ends_with("but it runs only in development or tests."),
        "{}",
        development.message
    );
}

#[test]
fn a_django_view_exempt_from_csrf_needs_the_check_to_name_it() {
    let project = Project::new();
    project.write(
        "shop/views.py",
        "from django.views.decorators.csrf import csrf_exempt\n\n\n@csrf_exempt\ndef update_address(request):\n    profile = request.user.profile\n    profile.address = request.POST['address']\n    profile.save()\n    return redirect('profile')\n",
    );
    let mut options = args();
    options.rules = vec![catalog::UNSAFE_SETTINGS.into()];
    let report = run_with_nouls(&project, &options, &[("weakened", 0.9), ("csrf", 0.95)]);
    let finding = &report.files[0].findings[0];
    assert_eq!(finding.strength, Strength::Review);
    assert_eq!(
        finding.category.as_deref(),
        Some("CWE-352 cross-site request forgery")
    );
    options.refresh = true;
    let report = run_with_nouls(&project, &options, &[("weakened", 0.9), ("csrf", 0.6)]);
    assert_eq!(report.files[0].findings[0].strength, Strength::Note);
}

#[test]
fn an_undecided_markup_check_in_a_view_is_settled_by_what_it_sends_back() {
    let project = Project::new();
    project.write(
        "shop/views.py",
        "from django.shortcuts import render\n\n\ndef detail(request, order_id):\n    order = Order.objects.get(pk=order_id)\n    title = 'Order ' + order.note\n    return render(request, 'orders/detail.html', {'title': title})\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let status = |output: &str, options: &CheckArgs| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("interpreted", noul_at(0.9)),
            ("markup", noul_at(0.4)),
            ("origin", spread(0.0, 0.3, 0.7)),
            (
                "markup_output",
                choice_of(output, &["escaped", "text", "raw", "none"]),
            ),
        ];
        run(&project, options, &mut eval).files[0].dimensions[catalog::INJECTION]
            .status
            .clone()
    };
    assert_eq!(status("escaped", &options), Status::Clear);
    options.refresh = true;
    assert_eq!(status("raw", &options), Status::Uncertain);
}

#[test]
fn django_error_views_middleware_and_the_rest_framework_handler_are_error_handlers() {
    let project = Project::new();
    project.write("shop/urls.py", URLS);
    project.write(
        "shop/views.py",
        "import traceback\n\nfrom django.http import HttpResponseServerError\n\n\ndef server_error(request):\n    return HttpResponseServerError(traceback.format_exc())\n",
    );
    project.write(
        "shop/middleware.py",
        "from django.http import JsonResponse\n\n\nclass Errors:\n    def process_exception(self, request, exception):\n        return JsonResponse({'error': str(exception)}, status=500)\n",
    );
    project.write(
        "shop/settings.py",
        "INSTALLED_APPS = ['shop']\nREST_FRAMEWORK = {\n    'EXCEPTION_HANDLER': 'shop.api.handle',\n    # 'EXCEPTION_HANDLER': 'shop.api.unused',\n}\n",
    );
    project.write(
        "shop/api.py",
        "from rest_framework.views import exception_handler\n\n\ndef handle(exc, context):\n    return exception_handler(exc, context)\n\n\ndef unused(exc, context):\n    return None\n",
    );
    let mut options = args();
    options.rules = vec![catalog::SENSITIVE_DATA.into()];
    let (_, plan) = planned(&project, &options);
    assert_eq!(
        registered_handlers(&plan),
        [
            "`'EXCEPTION_HANDLER': 'shop.api.handle',` (shop/settings.py:3)",
            "`Django middleware Errors.process_exception` (shop/middleware.py:5)",
            "`handler500 = 'shop.views.server_error'` (shop/urls.py:10)",
        ],
        "the commented-out handler registers nothing"
    );
    let handler = plan
        .requests
        .iter()
        .find(|p| p.request["state"]["error_handler"].is_object())
        .unwrap();
    assert!(
        handler.request["questions"]["handler_leaks"]["instructions"]["question"]
            .as_str()
            .unwrap()
            .contains("framework's errors written for the user")
    );
}

#[test]
fn a_django_injection_note_that_no_check_found_is_settled_like_an_undecided_unit() {
    let project = Project::new();
    project.write(
        "shop/views.py",
        "from django.shortcuts import redirect\n\n\ndef task_edit(request, project_id, task_id):\n    task = Task.objects.get(pk=task_id)\n    task.title = request.POST.get('title')\n    task.save()\n    return redirect('/shop/' + project_id + '/' + task_id)\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let outcome = |target: &str, options: &CheckArgs| {
        let mut eval = scripted(0);
        eval.overrides = vec![
            ("resource", noul_at(0.9)),
            ("redirect", noul_at(0.3)),
            ("origin", spread(0.0, 0.05, 0.95)),
            (
                "redirect_target",
                choice_of(target, &["own", "checked", "given", "outside", "none"]),
            ),
        ];
        let report = run(&project, options, &mut eval);
        let file = &report.files[0];
        (
            file.dimensions[catalog::INJECTION].status.clone(),
            file.findings.iter().map(|f| f.strength).collect::<Vec<_>>(),
        )
    };
    assert_eq!(outcome("own", &options), (Status::Clear, Vec::new()));
    options.refresh = true;
    assert_eq!(
        outcome("outside", &options).1,
        [Strength::Note],
        "a target the Choice does not clear keeps the note"
    );
}
