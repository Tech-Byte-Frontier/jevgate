//! Spring MVC routes and the requests Java tests send to them. A MockMvc or
//! RestTemplate test calls its controller through a path, not by name, so
//! without the route the test named no code under test.
use super::text;
use tree_sitter::Node;

/// An HTTP method (lowercase, empty for any) and a path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Route {
    pub method: String,
    pub path: String,
}

/// Spring mapping annotations and the method each one maps; `RequestMapping`
/// names its methods in a `method` attribute, or maps them all.
const MAPPINGS: &[(&str, &str)] = &[
    ("GetMapping", "get"),
    ("PostMapping", "post"),
    ("PutMapping", "put"),
    ("DeleteMapping", "delete"),
    ("PatchMapping", "patch"),
    ("RequestMapping", ""),
];

/// Calls that send a test request: MockMvc's `get("/owners")` and
/// RestTemplate's `RequestEntity.get(…)`, `getForEntity(…)` and the like.
const REQUESTS: &[(&str, &str)] = &[
    ("get", "get"),
    ("post", "post"),
    ("put", "put"),
    ("delete", "delete"),
    ("patch", "patch"),
    ("getForEntity", "get"),
    ("getForObject", "get"),
    ("postForEntity", "post"),
    ("postForObject", "post"),
];

/// The routes a Java controller method maps: its mapping annotations' paths
/// under the enclosing class's `@RequestMapping` prefix.
pub fn spring(method: Node<'_>, source: &str) -> Vec<Route> {
    if method.kind() != "method_declaration" {
        return Vec::new();
    }
    let prefixes = method
        .parent()
        .filter(|body| body.kind() == "class_body")
        .and_then(|body| body.parent())
        .map(|class| mappings(class, source))
        .and_then(|found| found.into_iter().next())
        .map_or_else(|| vec![String::new()], |(_, paths)| paths);
    let mut routes = Vec::new();
    for (verb, paths) in mappings(method, source) {
        for prefix in &prefixes {
            for path in &paths {
                routes.push(Route {
                    method: verb.clone(),
                    path: format!("{prefix}{path}"),
                });
            }
        }
    }
    routes
}

/// The mapping annotations on a declaration: each one's method and paths
/// (one empty path when it names none).
fn mappings(declaration: Node<'_>, source: &str) -> Vec<(String, Vec<String>)> {
    let mut found = Vec::new();
    let mut cursor = declaration.walk();
    for modifiers in declaration
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "modifiers")
    {
        let mut inner = modifiers.walk();
        for annotation in modifiers.named_children(&mut inner) {
            let Some(name) = annotation
                .child_by_field_name("name")
                .map(|n| text(n, source).rsplit('.').next().unwrap_or(""))
            else {
                continue;
            };
            let Some(&(_, verb)) = MAPPINGS.iter().find(|(mapping, _)| *mapping == name) else {
                continue;
            };
            let mut verb = verb.to_string();
            let mut paths = Vec::new();
            if let Some(arguments) = annotation.child_by_field_name("arguments") {
                let mut args = arguments.walk();
                for argument in arguments.named_children(&mut args) {
                    if argument.kind() != "element_value_pair" {
                        strings(argument, source, &mut paths);
                        continue;
                    }
                    let key = argument
                        .child_by_field_name("key")
                        .map_or("", |k| text(k, source));
                    let Some(value) = argument.child_by_field_name("value") else {
                        continue;
                    };
                    match key {
                        "value" | "path" => strings(value, source, &mut paths),
                        "method" => {
                            let named = text(value, source);
                            verb = named
                                .rsplit('.')
                                .next()
                                .unwrap_or("")
                                .trim_end_matches('}')
                                .trim()
                                .to_lowercase();
                            if named.contains(',') {
                                verb.clear();
                            }
                        }
                        _ => {}
                    }
                }
            }
            if paths.is_empty() {
                paths.push(String::new());
            }
            found.push((verb, paths));
        }
    }
    found
}

/// String literals in an annotation value: one path or an array of them.
fn strings(value: Node<'_>, source: &str, paths: &mut Vec<String>) {
    match value.kind() {
        "string_literal" => paths.push(text(value, source).trim_matches('"').to_string()),
        "element_value_array_initializer" => {
            let mut cursor = value.walk();
            for item in value.named_children(&mut cursor) {
                strings(item, source, paths);
            }
        }
        _ => {}
    }
}

/// A request a test sends: `get("/owners/{id}", 1)` and the like, whose first
/// argument is a literal path.
pub fn request(call: Node<'_>, source: &str) -> Option<Route> {
    if call.kind() != "method_invocation" {
        return None;
    }
    let name = text(call.child_by_field_name("name")?, source);
    let &(_, verb) = REQUESTS.iter().find(|(request, _)| *request == name)?;
    let first = call.child_by_field_name("arguments")?.named_child(0)?;
    let path = text(first, source).trim_matches('"');
    (first.kind() == "string_literal" && path.starts_with('/')).then(|| Route {
        method: verb.to_string(),
        path: path.to_string(),
    })
}

impl Route {
    /// How well this route serves a request: `None` when it does not, else
    /// the number of segments it names literally. The method must match,
    /// unless the route maps any; a route's `{variable}` matches any one
    /// segment, and a request's `{variable}` (a template filled in by the
    /// test) only a route's variable. The query is ignored. Spring picks the
    /// most literal route, so `/owners/new` is not `/owners/{ownerId}`.
    pub fn serves(&self, request: &Route) -> Option<usize> {
        let segments = |path: &str| -> Vec<String> {
            path.split(['?', '#'])
                .next()
                .unwrap_or("")
                .split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        };
        let variable = |s: &str| s.starts_with('{') && s.ends_with('}');
        let (route, sent) = (segments(&self.path), segments(&request.path));
        ((self.method.is_empty() || self.method == request.method)
            && route.len() == sent.len()
            && route.iter().zip(&sent).all(|(a, b)| variable(a) || a == b))
        .then(|| route.iter().filter(|a| !variable(a)).count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const CONTROLLER: &str = "@Controller\n@RequestMapping(\"/owners/{ownerId}\")\nclass PetController {\n\t@GetMapping(\"/pets/new\")\n\tpublic String initCreationForm() {\n\t\treturn VIEW;\n\t}\n\n\t@PostMapping({\"/pets/new\", \"/pets/add\"})\n\tpublic String processCreationForm(Pet pet) {\n\t\treturn \"redirect:/owners/{ownerId}\";\n\t}\n\n\t@RequestMapping(path = \"/pets/{petId}\", method = RequestMethod.DELETE)\n\tpublic String remove() {\n\t\treturn VIEW;\n\t}\n\n\tvoid helper() {}\n}\n";

    fn routes_of(source: &str) -> Vec<(String, Vec<Route>)> {
        let tree = crate::syntax::parse(Path::new("PetController.java"), source)
            .unwrap()
            .unwrap();
        let mut found = Vec::new();
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.kind() == "method_declaration" {
                let name = text(node.child_by_field_name("name").unwrap(), source);
                found.push((name.to_string(), spring(node, source)));
            }
            let mut cursor = node.walk();
            stack.extend(node.named_children(&mut cursor));
        }
        found.sort();
        found
    }

    fn route(method: &str, path: &str) -> Route {
        Route {
            method: method.into(),
            path: path.into(),
        }
    }

    #[test]
    fn controller_methods_map_their_paths_under_the_class_prefix() {
        assert_eq!(
            routes_of(CONTROLLER),
            [
                ("helper".to_string(), vec![]),
                (
                    "initCreationForm".to_string(),
                    vec![route("get", "/owners/{ownerId}/pets/new")]
                ),
                (
                    "processCreationForm".to_string(),
                    vec![
                        route("post", "/owners/{ownerId}/pets/new"),
                        route("post", "/owners/{ownerId}/pets/add")
                    ]
                ),
                (
                    "remove".to_string(),
                    vec![route("delete", "/owners/{ownerId}/pets/{petId}")]
                ),
            ]
        );
    }

    #[test]
    fn a_request_is_served_by_the_route_of_its_method_and_segments() {
        let form = route("get", "/owners/{ownerId}/pets/new");
        assert_eq!(
            form.serves(&route("get", "/owners/{ownerId}/pets/new")),
            Some(3)
        );
        assert_eq!(
            form.serves(&route("get", "/owners/1/pets/new?x=1")),
            Some(3)
        );
        assert_eq!(form.serves(&route("post", "/owners/1/pets/new")), None);
        assert_eq!(form.serves(&route("get", "/owners/1/pets")), None);
        assert_eq!(
            route("", "/owners").serves(&route("post", "/owners")),
            Some(1)
        );
        // A filled-in template reaches only a route variable.
        let details = route("get", "/owners/{ownerId}");
        assert_eq!(details.serves(&route("get", "/owners/{id}")), Some(1));
        assert_eq!(
            route("get", "/owners/new").serves(&route("get", "/owners/{id}")),
            None
        );
        assert_eq!(details.serves(&route("get", "/owners/new")), Some(1));
        assert_eq!(
            route("get", "/owners/new").serves(&route("get", "/owners/new")),
            Some(2)
        );
        let test = "class T {\n\tvoid t() throws Exception {\n\t\tmockMvc.perform(post(\"/owners/{ownerId}/pets/new\", 1).param(\"name\", \"Betty\"));\n\t\ttemplate.exchange(RequestEntity.get(\"/owners/1\").build(), String.class);\n\t\tlog.get(\"key\");\n\t}\n}\n";
        let tree = crate::syntax::parse(Path::new("T.java"), test)
            .unwrap()
            .unwrap();
        let mut sent = Vec::new();
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            sent.extend(request(node, test));
            let mut cursor = node.walk();
            stack.extend(node.named_children(&mut cursor));
        }
        sent.sort();
        assert_eq!(
            sent,
            [
                route("get", "/owners/1"),
                route("post", "/owners/{ownerId}/pets/new")
            ]
        );
    }
}
