//! A compact, offline view of recorded judgments; no source reads or inference.
use crate::schema::{Dimension, Report};
use anyhow::Result;
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Stdio},
};

fn dimension(rule: &str, d: &Dimension) -> Value {
    json!({"rule":rule,"status":d.status,"detail":d.decision_basis,
        "probability":d.concern_probability,"units":d.units})
}

fn batch_cost(report: &Report) -> Option<Value> {
    crate::output::estimated_usd(report).map(|usd| {
        json!({
            "estimated_usd": usd,
            "input_per_million": crate::output::INPUT_USD_PER_MILLION,
            "output_per_million": 0.0,
            "checked_at": crate::output::PRICE_CHECKED
        })
    })
}

pub fn render(report: &Report) -> Result<String> {
    let files: Vec<_> = report
        .files
        .iter()
        .map(|file| {
            let checks: Vec<_> = file
                .dimensions
                .iter()
                .map(|(rule, d)| dimension(rule, d))
                .collect();
            json!({"path":file.path,"status":file.status,"cached":file.cached,"checks":checks,
            "findings":file.findings,"limitations":file.context_limitations,
            "error":file.error,
            "classification":file.classification.as_ref().and_then(|class| {
                (class.reason.as_str() != file.error.as_deref().unwrap_or("")).then_some(class.reason.clone())
            })})
        })
        .collect();
    let rules: Vec<_> = crate::catalog::rules()
        .into_iter()
        .map(|r| json!({"key":r.key,"id":r.id,"description":r.inspection}))
        .collect();
    let data = json!({"root":report.root,"generation":report.generation,"generated_at":report.generated_at,
        "status":report.status,"complete":report.complete,"settled":report.settled,
        "refresh":report.watcher_pid.is_some(),"model":report.requested_model,
        "requests":report.api_requests,"tokens":report.paid_input_tokens,
        "cost":batch_cost(report),"gate":report.gate,"fail_on":report.fail_on,
        "errors":report.errors,"deleted":report.deleted_files,"files":files,"rules":rules});
    // Even a filename or analyzer message may contain </script>. Never let data
    // terminate the JSON element, and insert all displayed strings with textContent.
    let data = serde_json::to_string(&data)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    Ok(include_str!("report.html").replacen("__JEVGATE_DATA__", &data, 1))
}

pub fn open(root: &Path) {
    let path = root.join(".jevgate/report.html");
    note!("JevGate report: {}", path.display());
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let program = "explorer.exe";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let program = "xdg-open";
    match Command::new(program)
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                if !child.wait().is_ok_and(|s| s.success()) {
                    note!(
                        "Could not open the browser. Open {} manually.",
                        path.display()
                    );
                }
            });
        }
        Err(_) => note!(
            "Could not open the browser. Open {} manually.",
            path.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dashboard_preserves_uncertainty_and_escapes_untrusted_text() {
        let p = crate::tests::Project::new();
        p.write("api.py", "def value():\n    return 1\n");
        let mut args = crate::tests::args();
        args.dry_run = true;
        let context = p.context();
        let scope = crate::inventory::scope(&args, &context).unwrap();
        let inputs = crate::inventory::collect(&args, &context, &scope).unwrap();
        let mut report = crate::evaluate::snapshot(
            &inputs,
            &Default::default(),
            &args,
            crate::evaluate::SnapshotContext {
                root: &p.0,
                generation: 1,
                requests: 0,
            },
        );
        report.files[0].status = crate::schema::Status::Uncertain;
        report.files[0].error = Some("</script><script>alert('x')</script>&".into());
        let html = render(&report).unwrap();
        assert!(!html.contains("<script>alert"));
        let data = html
            .split("<script id=\"data\" type=\"application/json\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap();
        let decoded: Value = serde_json::from_str(data).unwrap();
        assert_eq!(decoded["files"][0]["status"], "uncertain");
        assert_eq!(
            decoded["files"][0]["error"],
            report.files[0].error.as_deref().unwrap()
        );
        assert!(decoded.get("initial_requests").is_none());
        assert!(
            decoded["files"][0]["findings"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(decoded["fail_on"], serde_json::json!(["review"]));
        assert!(!html.contains("id=\"root\""));
        report.paid_input_tokens = 1_000_000;
        report.paid_output_tokens = 12_345;
        let cost = batch_cost(&report).unwrap();
        assert!((cost["estimated_usd"].as_f64().unwrap() - 0.042).abs() < 1e-12);
        report.paid_input_tokens = 0;
        assert_eq!(batch_cost(&report).unwrap()["estimated_usd"], 0.0);
        report.requested_model = "unknown-model".into();
        assert!(batch_cost(&report).is_none());
        assert!(!html.contains("def value()"));
    }
}
