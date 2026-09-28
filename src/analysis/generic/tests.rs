use super::*;

#[test]
fn every_query_compiles_and_every_kind_the_table_names_is_its_grammar_s() {
    for language in &LANGUAGES {
        let grammar = language.grammar();
        language.query();
        let kinds = [
            language.blocks,
            language.control,
            language.conditionals,
            language.clauses,
            language.literals,
        ];
        for kind in kinds.into_iter().flatten() {
            assert_ne!(
                grammar.id_for_node_kind(kind, true),
                0,
                "{}: {kind}",
                language.name
            );
        }
        for conditional in language.conditionals {
            assert!(language.control.contains(conditional), "{conditional}");
        }
    }
}

#[test]
fn each_extension_names_one_language_and_none_of_the_other_grammars() {
    let mut seen = std::collections::BTreeSet::new();
    for language in &LANGUAGES {
        for extension in language.extensions {
            assert!(seen.insert(*extension), "{extension}");
            let path = format!("file.{extension}");
            assert_eq!(of(Path::new(&path)).unwrap().name, language.name);
        }
    }
    for other in [
        "rs", "py", "ts", "go", "java", "cs", "rb", "php", "bend", "zig", "zsh",
    ] {
        assert!(of(Path::new(&format!("file.{other}"))).is_none(), "{other}");
    }
    assert_eq!(of(Path::new("Main.KT")).unwrap().name, "Kotlin");
    assert_eq!(
        family(Path::new("editor.h")),
        family(Path::new("editor.cpp"))
    );
    assert_ne!(family(Path::new("App.kt")), family(Path::new("App.swift")));
    assert_eq!(family(Path::new("app.rs")), None);
}

#[test]
fn test_files_are_found_by_each_language_s_conventions() {
    for path in [
        "app/src/androidTest/kotlin/LoginTest.kt",
        "shared/src/commonTest/kotlin/Api.kt",
        "server/src/main/kotlin/OrdersTest.kt",
        "Tests/VaporTests/Utilities/Checkpoint.swift",
        "Sources/AppUITests/Launch.swift",
        "Sources/App/RouterTests.swift",
        "core/src/main/scala/CartIT.scala",
        "spec/cart_spec.lua",
        "lua/cart_spec.lua",
        "integration_test/app.dart",
        "test/deploy.bats",
        "src/parser_unittest.cc",
        "test.c",
        "testheap.c",
        "linenoise-test.c",
        "src/Test.cpp",
    ] {
        assert!(test_path(Path::new(path)), "{path}");
    }
    for path in [
        "app/src/main/kotlin/Contest.kt",
        "app/src/latest/kotlin/Api.kt",
        // Production code takes Kotest's and ScalaTest's names too.
        "app/shared/src/commonMain/kotlin/utils/AnimatedContentSpec.kt",
        "core/src/main/scala/CartSuite.scala",
        "Sources/Vapor/Test.swift",
        "lib/spectrum.lua",
        "src/unittest.c",
        "src/latest.c",
        "src/app/OrdersTest.java",
        "scripts/deploy.sh",
    ] {
        assert!(!test_path(Path::new(path)), "{path}");
    }
}

#[test]
fn a_header_holding_cpp_code_is_read_as_cpp_and_a_c_header_as_c() {
    let cpp = "#ifndef CACHE_H_\n#define CACHE_H_\n\nnamespace store {\nclass Cache;\n}  // namespace store\n\n#endif\n";
    let c = "#ifndef UTIL_H\n#define UTIL_H\n\n#ifdef __cplusplus\nextern \"C\" {\n#endif\n\nint add(int a, int b);\n\n#ifdef __cplusplus\n}\n#endif\n#endif\n";
    assert_eq!(read(Path::new("include/cache.h"), cpp).unwrap().name, "C++");
    assert_eq!(read(Path::new("include/util.h"), c).unwrap().name, "C");
    // Only a header's code decides: a `.c` file is C whatever it holds.
    assert_eq!(read(Path::new("src/cache.c"), cpp).unwrap().name, "C");
    assert_eq!(family(Path::new("include/cache.h")), Some("C"));
}
