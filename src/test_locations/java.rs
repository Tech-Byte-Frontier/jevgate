//! Java tests: JUnit and TestNG test methods by annotation, JUnit 3
//! `TestCase` classes, and the classes that hold tests.
use super::child_text;
use tree_sitter::Node;

/// Annotations that make a Java method a test: JUnit 4 and 5, TestNG and jqwik.
const JAVA_TEST_ANNOTATIONS: &[&str] = &[
    "Test",
    "ParameterizedTest",
    "RepeatedTest",
    "TestFactory",
    "TestTemplate",
    "Theory",
    "Property",
];

/// JUnit lifecycle annotations: only a test class has these methods.
const JAVA_LIFECYCLE_ANNOTATIONS: &[&str] = &[
    "BeforeEach",
    "AfterEach",
    "BeforeAll",
    "AfterAll",
    "Before",
    "After",
    "BeforeClass",
    "AfterClass",
    "BeforeMethod",
    "AfterMethod",
];

/// A Java test method: annotated `@Test`, `@ParameterizedTest` and the like,
/// by simple or qualified name, or by a composed annotation whose name ends
/// in `Test`, such as a project's `@MultiLocaleTest`.
pub(crate) fn java_test_method(node: Node<'_>, source: &str) -> bool {
    java_annotated(node, source, |name| {
        JAVA_TEST_ANNOTATIONS.contains(&name) || name.ends_with("Test")
    })
}

/// A Java method whose annotations include one `matches` accepts, by simple name.
fn java_annotated(node: Node<'_>, source: &str, matches: impl Fn(&str) -> bool) -> bool {
    node.kind() == "method_declaration" && {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .filter(|c| c.kind() == "modifiers")
            .any(|modifiers| {
                let mut inner = modifiers.walk();
                modifiers.named_children(&mut inner).any(|annotation| {
                    matches!(annotation.kind(), "marker_annotation" | "annotation")
                        && annotation.child_by_field_name("name").is_some_and(|name| {
                            matches(child_text(name, source).rsplit('.').next().unwrap_or(""))
                        })
                })
            })
    }
}

/// A JUnit 3 test class: one that extends `TestCase`, whose `test…` methods are tests.
pub(crate) fn junit3_class(node: Node<'_>, source: &str) -> bool {
    node.kind() == "class_declaration"
        && node
            .child_by_field_name("superclass")
            .is_some_and(|s| child_text(s, source).ends_with("TestCase"))
}

/// A Java class that holds tests: a test or lifecycle method (`@BeforeEach`,
/// as a subclass of a test class has) among its members, a nested test class
/// (JUnit 5 `@Nested`), or JUnit 3's `TestCase` as its superclass.
pub(super) fn java_test_class(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "class_declaration" {
        return false;
    }
    let Some(body) = node
        .child_by_field_name("body")
        .filter(|b| b.kind() == "class_body")
    else {
        return false;
    };
    let mut cursor = body.walk();
    junit3_class(node, source)
        || body.named_children(&mut cursor).any(|member| {
            java_test_method(member, source)
                || java_annotated(member, source, |name| {
                    JAVA_LIFECYCLE_ANNOTATIONS.contains(&name)
                })
                || java_test_class(member, source)
        })
}
