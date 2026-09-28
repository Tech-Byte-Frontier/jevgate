//! Server templates: inline scripts, the view a Node handler renders, and
//! template code that writes client data unescaped.
use super::*;

#[test]
fn a_server_template_is_judged_by_its_inline_scripts_only() {
    let project = Project::new();
    project.write(
        "app/views/sessions/new.html.erb",
        "<h1><%= t('login') %></h1>\n<script>\n  var param = location.hash.split('#')[1];\n  document.write('<div>' + decodeURIComponent(param) + '</div>');\n</script>\n",
    );
    project.write(
        "app/views/users/show.html.erb",
        "<p><%= raw @user.bio %></p>\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let (inputs, plan) = planned(&project, &options);
    let paths: Vec<_> = inputs.iter().map(|i| i.result.path.clone()).collect();
    assert_eq!(
        paths,
        [std::path::PathBuf::from("app/views/sessions/new.html.erb")],
        "a template without inline scripts is not selected"
    );
    let request = &plan.requests[0].request;
    assert_eq!(request["state"]["file"]["language"], "JavaScript");
    assert!(
        request["state"]["file"]["framework"]
            .as_str()
            .unwrap()
            .contains("runs in the visitor's browser")
    );
    let page = &request["state"]["functions"][0];
    assert_eq!(page["name"], "top-level code");
    assert!(
        page["source"].as_str().unwrap().contains("document.write("),
        "{page}"
    );
}

#[test]
fn a_node_handler_is_sent_the_unescaped_lines_of_the_view_it_renders() {
    let project = Project::new();
    project.write(
        "app.js",
        "const express = require('express');\nconst app = express();\n\nfunction search(req, res) {\n  const term = req.query.q;\n  res.render('shop/products', { term });\n}\n\napp.get('/search', search);\n",
    );
    project.write(
        "views/shop/products.ejs",
        "<%- include('../head') %>\n<p>Results for <%- term %></p>\n<p><%= term %></p>\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into()];
    let (_, plan) = planned(&project, &options);
    let first = &plan.requests[0].request;
    let search = &first["state"]["functions"][0];
    assert_eq!(
        search["templates_it_renders_that_write_values_without_escaping"],
        json!([{"template": "views/shop/products.ejs", "unescaped_output": ["2: <p>Results for <%- term %></p>"]}])
    );
    let interpreted = first["questions"]["f0_interpreted"]["criteria"]["true"]
        .as_str()
        .unwrap();
    assert!(
        interpreted.contains("passed to a template that writes it unescaped"),
        "{interpreted}"
    );
    let unit = plan.files[&0]
        .units
        .iter()
        .find(|u| u.rule == catalog::INJECTION)
        .unwrap();
    let Detail::Security {
        trace: Some(trace), ..
    } = &unit.detail
    else {
        panic!("a traced injection unit");
    };
    let markup = trace.request()["questions"]["markup"]["instructions"]["question"].clone();
    assert!(
        markup
            .as_str()
            .unwrap()
            .contains("through a template it renders"),
        "{markup}"
    );
}

#[test]
fn a_template_writing_client_data_unescaped_is_judged_as_template_code() {
    let project = Project::new();
    project.write(
        "app/views/layouts/application.html.erb",
        "<html>\n<style>body { font-size: <%= raw cookies[:font] %>; }</style>\n<p><%= @title %></p>\n</html>\n",
    );
    let mut options = args();
    options.rules = vec![catalog::INJECTION.into(), catalog::SENSITIVE_DATA.into()];
    let (inputs, plan) = planned(&project, &options);
    assert_eq!(inputs.len(), 1, "selected for its template code alone");
    let names: Vec<(&str, &str)> = plan.files[&0]
        .units
        .iter()
        .map(|u| (u.rule, u.name.as_str()))
        .collect();
    assert_eq!(
        names,
        [(catalog::INJECTION, "template code")],
        "an ERB tag is judged for what it writes"
    );
    let code = &plan.requests[0].request["state"]["functions"][0];
    assert_eq!(code["source"], "<%= raw cookies[:font] %>");
    assert_eq!(plan.files[&0].units[0].locations[0].start_line, 2);
}

#[test]
fn template_code_is_not_asked_without_a_rule_that_judges_it() {
    let project = Project::new();
    project.write(
        "app/views/layouts/application.html.erb",
        "<html>\n<style>body { font-size: <%= raw cookies[:font] %>; }</style>\n</html>\n",
    );
    let mut options = args();
    options.rules = vec![
        catalog::SENSITIVE_DATA.into(),
        catalog::UNSAFE_SETTINGS.into(),
    ];
    let (_, plan) = planned(&project, &options);
    assert!(
        plan.requests.is_empty(),
        "only injection judges an ERB page's template code, so nothing is asked"
    );
    assert!(plan.files.values().all(|f| f.units.is_empty()));
}
