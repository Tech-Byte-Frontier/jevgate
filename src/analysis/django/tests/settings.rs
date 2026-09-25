//! Settings modules, the settings they assign and the secrets they hold.
use super::*;

#[test]
fn settings_modules_are_named_by_what_they_assign_and_where_they_live() {
    let project = "INSTALLED_APPS = ['app']\nDEBUG = True\n";
    let dev = "from .base import *\n\nDEBUG = True\nALLOWED_HOSTS = ['*']\n";
    let constants = "DEBUG = False\nLIMIT = 10\n";
    let check = |path: &str, source: &str| {
        settings_module(Path::new(path), tree(source).root_node(), source)
    };
    assert!(check("project/config.py", project));
    assert!(check("project/settings/dev.py", dev));
    assert!(check("project/settings.py", constants));
    assert!(
        !check("project/app/constants.py", constants),
        "a DEBUG flag elsewhere"
    );
    assert!(!check("project/settings/limits.py", "LIMIT = 10\n"));
    assert!(!check("project/settings.ts", project));
}

#[test]
fn secret_literals_are_redacted_but_names_and_other_values_stay() {
    let source = "SECRET_KEY = 'abc123'\nDATABASES = {'default': {'PASSWORD': 'hunter2', 'NAME': 'db', 'USER': ''}}\nKEY = os.environ['KEY']\nDEBUG = True\n";
    let tree = tree(source);
    let mut found = Vec::new();
    secret_literals(tree.root_node(), source, &mut found);
    assert_eq!(found.len(), 2, "empty and non-literal values stay");
    assert_eq!(
        redacted(source, 0..source.len(), &found),
        "SECRET_KEY = '<redacted 6-character literal>'\nDATABASES = {'default': {'PASSWORD': '<redacted 7-character literal>', 'NAME': 'db', 'USER': ''}}\nKEY = os.environ['KEY']\nDEBUG = True\n"
    );
    let key = source.find("\nKEY =").unwrap() + 1;
    let line = key..key + source[key..].find('\n').unwrap();
    assert_eq!(
        redacted(source, line.clone(), &found),
        &source[line],
        "a statement without secrets is shown as written"
    );
}

#[test]
fn setting_names_are_upper_case() {
    assert!(setting_name("SESSION_COOKIE_SECURE"));
    assert!(setting_name("X2"));
    assert!(!setting_name("debug"));
    assert!(!setting_name("_"));
    assert!(security_setting("SESSION_COOKIE_HTTPONLY"));
    assert!(!security_setting("LANGUAGE_CODE"));
    assert!(secret_name("AWS_SECRET_ACCESS_KEY"));
    assert!(!secret_name("LANGUAGE_CODE"));
}

#[test]
fn settings_extend_the_modules_they_star_import() {
    let production = "from .base import *  # noqa\nfrom config.settings.cors import *\nimport os\n";
    assert!(extends(production, Path::new("site/settings/base.py")));
    assert!(extends(production, Path::new("config/settings/cors.py")));
    assert!(!extends(production, Path::new("site/settings/os.py")));
    assert!(!extends(
        "from .base import Base\n",
        Path::new("site/settings/base.py")
    ));
}

#[test]
fn module_constants_are_shown_with_secrets_redacted() {
    let source = "import os\n\nINVOICE_DIR = os.path.join(BASE_DIR, 'invoices')\nAPI_TOKEN = 's3cr3t-value'\nlimit = 3\n\ndef f():\n    X = 1\n";
    assert_eq!(
        module_constants(tree(source).root_node(), source),
        [
            (
                "INVOICE_DIR".to_string(),
                "INVOICE_DIR = os.path.join(BASE_DIR, 'invoices')".to_string()
            ),
            (
                "API_TOKEN".to_string(),
                "API_TOKEN = '<redacted 12-character literal>'".to_string()
            ),
        ]
    );
}

#[test]
fn a_dotted_path_under_a_secret_name_is_not_a_secret() {
    let source = "JWT_AUTH = {'JWT_GET_USER_SECRET_KEY': 'app.auth.services.user_secret_key', 'JWT_SECRET_KEY': 'k3y!'}\n";
    let tree = tree(source);
    let mut found = Vec::new();
    secret_literals(tree.root_node(), source, &mut found);
    assert_eq!(
        redacted(source, 0..source.len(), &found),
        "JWT_AUTH = {'JWT_GET_USER_SECRET_KEY': 'app.auth.services.user_secret_key', 'JWT_SECRET_KEY': '<redacted 4-character literal>'}\n"
    );
}
