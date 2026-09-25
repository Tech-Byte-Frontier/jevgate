//! PHP: functions, methods, registered and returned closures.
use super::*;

#[test]
fn php_functions_methods_closures_and_their_facts_are_units() {
    let source = "<?php\nnamespace App\\Store;\n\nuse App\\Domain\\User\\UserRepository;\nuse Psr\\Log\\{LoggerInterface, NullLogger as Quiet};\n\nconst MAX_ROWS = 500;\n\n/** Finds a user. */\nfunction find_user($db, $name) {\n    $sql = \"SELECT * FROM users WHERE name = '$name'\";\n    if ($name === '') {\n        throw new InvalidArgumentException('empty name');\n    } elseif ($name === 'root') {\n        return null;\n    } elseif ($name === 'admin') {\n        return null;\n    }\n    foreach ([1, 2] as $i) {\n        $db->query($sql);\n    }\n    return Row::from(new Cursor($db));\n}\n\nabstract class Store extends Base {\n    public function __construct(private PDO $pdo) {}\n    public function load(int $id): ?User { return $this->pdo->prepare('x')->execute([$id]); }\n}\ntrait Cached { public function flush() { cache_clear(); } }\ninterface Finder { public function find(int $id): ?array; }\nenum Suit: string { case Hearts = 'H'; }\n$app->get('/users/{id}', function (Request $request, Response $response) {\n    return $response;\n});\nRoute::post('/pages', fn () => save());\n$handler = function ($e) { report($e); };\n";
    let file = parse(Path::new("store.php"), source).unwrap();
    let named: Vec<(&str, Kind, usize)> = file
        .units
        .iter()
        .map(|u| (u.name.as_str(), u.kind, u.line))
        .collect();
    assert_eq!(
        named,
        [
            ("find_user", Kind::Function, 10),
            ("Store::__construct", Kind::Method, 26),
            ("Store::load", Kind::Method, 27),
            ("Cached::flush", Kind::Method, 29),
            ("Finder", Kind::Type, 30),
            ("Suit", Kind::Type, 31),
            ("$app->get('/users/{id}')", Kind::Function, 32),
            ("Route::post('/pages')", Kind::Function, 35),
            ("$handler", Kind::Function, 36),
        ]
    );
    let find = &file.units[0];
    assert_eq!(find.doc, "Finds a user.");
    for callee in ["query", "from", "Cursor", "InvalidArgumentException"] {
        assert!(find.calls.contains(callee), "{callee}");
    }
    assert_eq!((find.nesting, find.branch_chain), (1, 3));
    assert_eq!(find.errors[0].error, "InvalidArgumentException");
    assert_eq!(find.errors[0].message, "'empty name'");
    assert!(find.literals.iter().any(|l| l.text == "'root'"));
    assert!(file.units[2].refs.contains("User"));
    assert_imports(&file, &["UserRepository", "LoggerInterface", "Quiet"]);
    assert_eq!(file.constants[0].name, "MAX_ROWS");
    assert!(crate::syntax::supported(Path::new("store.php")));
    let config = parse(
        Path::new("app/routes.php"),
        "<?php\nreturn function (App $app) {\n    $app->get('/', fn () => home());\n};\n",
    )
    .unwrap();
    assert_eq!(config.units[0].name, "returned closure");
    assert!(config.units[0].calls.contains("get"));
}
