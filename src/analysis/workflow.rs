//! GitHub Actions workflows for the workflow rule: the triggers, top-level
//! permissions and jobs of a file, found by indentation, and the `${{ }}`
//! expressions written inside each job's `run` scripts. These are candidates
//! and locations; Jev judges what an expression can hold.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub source: String,
    /// Distinct expressions inside `run` scripts, with the line of the first.
    pub run_expressions: Vec<(usize, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workflow {
    pub triggers: Option<String>,
    pub permissions: Option<String>,
    pub jobs: Vec<Job>,
}

impl Workflow {
    /// Whether a trigger runs with the base repository's secrets on events
    /// that other people start.
    pub fn privileged_trigger(&self) -> bool {
        self.triggers
            .as_deref()
            .is_some_and(|t| t.contains("pull_request_target") || t.contains("workflow_run"))
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn meaningful(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with('#')
}

/// The mapping key a line starts at this indentation, if any.
fn key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start_matches(' ');
    let end = trimmed.find(':')?;
    let key = trimmed[..end]
        .trim()
        .trim_matches(|c| c == '"' || c == '\'');
    (!key.is_empty()
        && key
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.')))
    .then_some(key)
}

/// Keys at `level` between `start` and `end`, each with its last line.
fn blocks(lines: &[&str], start: usize, end: usize, level: usize) -> Vec<(String, usize, usize)> {
    let mut found: Vec<(String, usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate().take(end).skip(start) {
        if !meaningful(line) {
            continue;
        }
        let depth = indent(line);
        if depth < level {
            break;
        }
        if depth == level
            && let Some(name) = key(line)
        {
            found.push((name.to_string(), i, i));
        } else if let Some(last) = found.last_mut() {
            last.2 = i;
        }
    }
    found
}

pub fn parse(text: &str) -> Workflow {
    let lines: Vec<&str> = text.lines().collect();
    let top = blocks(&lines, 0, lines.len(), 0);
    let source = |names: &[&str]| {
        top.iter()
            .find(|(k, ..)| names.contains(&k.as_str()))
            .map(|(_, a, b)| lines[*a..=*b].join("\n"))
    };
    let mut workflow = Workflow {
        // YAML 1.1 reads a bare `on` key as `true`.
        triggers: source(&["on", "true"]),
        permissions: source(&["permissions"]),
        jobs: Vec::new(),
    };
    let Some((_, first, last)) = top.iter().find(|(k, ..)| k == "jobs") else {
        return workflow;
    };
    let Some(level) = lines[first + 1..=*last]
        .iter()
        .find(|l| meaningful(l))
        .map(|l| indent(l))
    else {
        return workflow;
    };
    for (name, a, b) in blocks(&lines, first + 1, last + 1, level) {
        let source = lines[a..=b].join("\n");
        let run_expressions = run_expressions(&lines[a..=b])
            .into_iter()
            .map(|(line, e)| (a + 1 + line, e))
            .collect();
        workflow.jobs.push(Job {
            name,
            start_line: a + 1,
            end_line: b + 1,
            source,
            run_expressions,
        });
    }
    workflow
}

/// Expressions inside `run` values: inline, or in the block that follows.
fn run_expressions(lines: &[&str]) -> Vec<(usize, String)> {
    let mut found: Vec<(usize, String)> = Vec::new();
    let mut block: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let depth = indent(line);
        let body = line.trim_start().trim_start_matches("- ").trim_start();
        if let Some(value) = body.strip_prefix("run:") {
            let value = value.trim();
            block = (value.is_empty() || value.starts_with('|') || value.starts_with('>'))
                .then_some(depth);
            push_expressions(value, i, &mut found);
            continue;
        }
        match block {
            Some(level) if meaningful(line) && depth <= level => block = None,
            Some(_) => push_expressions(line, i, &mut found),
            None => {}
        }
    }
    found
}

fn push_expressions(text: &str, line: usize, found: &mut Vec<(usize, String)>) {
    let mut rest = text;
    while let Some(open) = rest.find("${{") {
        let after = &rest[open + 3..];
        let Some(close) = after.find("}}") else {
            return;
        };
        let expression = after[..close].trim().to_string();
        if !found.iter().any(|(_, e)| *e == expression) {
            found.push((line, expression));
        }
        rest = &after[close + 2..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKFLOW: &str = "name: Greet\non:\n  pull_request_target:\n    types: [opened]\npermissions:\n  contents: read\njobs:\n  greet:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - run: echo \"${{ github.event.pull_request.title }}\"\n      - name: Branch\n        run: |\n          git fetch origin ${{ github.head_ref }}\n          echo ${{ github.sha }}\n        env:\n          TITLE: ${{ github.event.pull_request.body }}\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: make\n";

    #[test]
    fn jobs_triggers_and_run_expressions_are_found_by_indentation() {
        let workflow = parse(WORKFLOW);
        assert!(workflow.privileged_trigger());
        assert_eq!(
            workflow.permissions.as_deref(),
            Some("permissions:\n  contents: read")
        );
        let names: Vec<_> = workflow
            .jobs
            .iter()
            .map(|j| (j.name.as_str(), j.start_line, j.end_line))
            .collect();
        assert_eq!(names, [("greet", 8, 18), ("build", 19, 22)]);
        assert_eq!(
            workflow.jobs[0].run_expressions,
            [
                (12, "github.event.pull_request.title".to_string()),
                (15, "github.head_ref".to_string()),
                (16, "github.sha".to_string()),
            ],
            "the env value is not inside a run script"
        );
        assert!(workflow.jobs[1].run_expressions.is_empty());
    }
}
