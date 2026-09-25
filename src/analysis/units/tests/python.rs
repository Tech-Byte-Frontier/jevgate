//! Python: methods of their class, decorated functions and docstrings.
use super::*;

const LOADER: &str = "import os\n\nclass Loader:\n    def load(self, name):\n        \"\"\"Read one file.\"\"\"\n        return os.path.join(name)\n\n@cache\ndef helper(value):\n    return value\n";

#[test]
fn python_methods_follow_their_class_and_decorated_functions_count() {
    assert_eq!(
        names("loader.py", LOADER),
        [
            ("Loader::load".into(), Kind::Method),
            ("helper".into(), Kind::Function)
        ]
    );
}

#[test]
fn a_python_docstring_is_the_unit_doc() {
    let load = parse(Path::new("loader.py"), LOADER)
        .unwrap()
        .units
        .remove(0);
    assert_eq!(load.doc, "Read one file.");
}
