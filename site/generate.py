#!/usr/bin/env python3
"""Write the site's reference pages from JevGate itself.

    generate.py JEVGATE SCHEMA OUT_DIR

rules.md comes from `jevgate rules --format json`, configuration.md from
jevgate.schema.json, and cli.md from each command's --help, so the pages
always describe the binary they were built with.
"""
import html
import json
import subprocess
import sys
from pathlib import Path

COMMANDS = ["auth", "check", "baseline", "rules", "init", "completions", "man", "serve", "mcp"]
GROUPS = {
    "maintainability": "On by default, except hardcoded values: add it with `--rule default --rule hardcoded-values`, or a level for it in `[rules]`.",
    "tests": "On by default. Test value and test redundancy are judged with `--include-tests` or `include_tests = true`; the laws of Bend 2 code are judged without it.",
    "security": "Opt-in: `--rule security`, or a level in `[rules]`.",
    "documentation": "Opt-in: `--rule documentation`, or a level in `[rules]`.",
}


def run(binary, *args):
    return subprocess.run([binary, *args], check=True, capture_output=True, text=True).stdout


def rules_page(binary):
    rules = json.loads(run(binary, "rules", "--format", "json"))
    version = run(binary, "--version").strip()
    lines = [
        "# Rules reference",
        "",
        f"Generated from `jevgate rules --format json` ({version}). A rule is named by its ID,",
        "its key or its group anywhere a rule is accepted: `--rule`, `--skip-rule`,",
        "`--fail-on TARGET=LEVEL`, `[rules]`, `[[scope]]` and `jevgate: allow(…)` comments.",
        "",
        "*Fails by default* names the levels that fail the check under the default gate level,",
        "`mature`: those right at least 80% of the time over at least 20 findings labeled by hand on",
        "projects JevGate was never tuned on; an opt-in rule's levels fail it once the rule is selected.",
        "The other findings are reported without failing it.",
        "*Reviews right* and *considers right* give the share of labeled findings that were right on",
        "those projects, and how many were labeled; a debatable one counts as not right.",
        "`tests/laws` is labeled only on Bend 2 projects, which these numbers leave out.",
        "",
        "| Rule | Key | Default | Fails by default | Reviews right | Considers right | Question |",
        "|---|---|---|---|---|---|---|",
    ]
    for rule in rules:
        anchor = rule["id"].replace("/", "-")
        default = "yes" if rule["default_enabled"] else "opt-in"
        maturity = rule["maturity"]
        lines.append(
            f"| [`{rule['id']}`](#{anchor}) | `{rule['key']}` | {default} | {blocks(maturity)}"
            f" | {right(maturity, 'review')} | {right(maturity, 'consider')} | {cell(rule['inspection'])} |"
        )
    group = None
    for rule in rules:
        if rule["group"] != group:
            group = rule["group"]
            lines += ["", f"## {group.capitalize()}", "", GROUPS.get(group, "")]
        anchor = rule["id"].replace("/", "-")
        lines += [
            "",
            f'<a id="{html.escape(anchor)}"></a>',
            f"### `{rule['id']}`",
            "",
            f"**Question:** {text(rule['inspection'])}",
            "",
            f"- **Key:** `{rule['key']}` · **Version:** {rule['version']}"
            + (" · **Needs tests:** yes" if rule["requires_tests"] else ""),
            f"- **Looks at:** {text(rule['scope'])}",
            f"- **Evidence unit:** {text(rule['unit'])}",
            f"- **Acceptable:** {text(rule['acceptable_example'])}",
        ]
    policy = rules[0]["decision_policy"]
    lines += [
        "",
        "## Decision policy",
        "",
        "Answers become findings in code, at the same thresholds for every rule:",
        "",
        "| Setting | Value |",
        "|---|---|",
    ]
    lines += [f"| `{name}` | {value:g} |" for name, value in sorted(policy.items())]
    return "\n".join(lines) + "\n"


def blocks(maturity):
    """The levels that fail the default gate, or "no"."""
    return ", ".join(level for level in ("review", "consider") if maturity.get(level, {}).get("mature")) or "no"


def right(maturity, level):
    """"87% of 23": labeled findings of a level right on unseen projects, or "-"."""
    unseen = maturity.get(level, {}).get("unseen")
    if not unseen or not unseen["labeled"]:
        return "-"
    percent = (200 * unseen["right"] + unseen["labeled"]) // (2 * unseen["labeled"])  # half up, as JevGate rounds
    return f"{percent}% of {unseen['labeled']}"


def configuration_page(schema_path):
    schema = json.loads(Path(schema_path).read_text())
    lines = [
        "# Configuration reference",
        "",
        "Generated from [`jevgate.schema.json`](https://github.com/Tech-Byte-Frontier/jevgate/blob/main/jevgate.schema.json),",
        "which is generated from the configuration types. [Configuration](../configuration.md) explains",
        "how the keys work together.",
        "",
        "| Key | Type | Meaning |",
        "|---|---|---|",
    ]
    for name, spec in sorted(schema["properties"].items()):
        lines.append(f"| `{name}` | {kind(spec, schema)} | {cell(spec.get('description', ''))} |")
    scope = schema["$defs"]["Scope"]
    lines += ["", "## `[[scope]]`", "", cell(scope.get("description", "")), "", "| Key | Type | Meaning |", "|---|---|---|"]
    for name, spec in sorted(scope["properties"].items()):
        lines.append(f"| `{name}` | {kind(spec, schema)} | {cell(spec.get('description', ''))} |")
    levels = schema["$defs"]["Level"]["anyOf"][0]["enum"]
    names = schema["$defs"]["Scope"]["properties"]["rules"]["propertyNames"]["enum"]
    lines += [
        "",
        "## Levels",
        "",
        ", ".join(f"`{level}`" for level in levels) + ". `off` is accepted in `[rules]` only.",
        "",
        "## Rule names",
        "",
        ", ".join(f"`{name}`" for name in names) + ".",
    ]
    return "\n".join(lines) + "\n"


def kind(spec, schema):
    if "$ref" in spec:
        return "rules list or table"
    if spec.get("type") == "array":
        items = spec.get("items", {})
        return "list of tables" if "$ref" in items else "list of " + items.get("type", "value") + "s"
    return spec.get("type", "value")


def cli_page(binary):
    lines = [
        "# Command-line reference",
        "",
        f"Generated from `--help` ({run(binary, '--version').strip()}). "
        "`jevgate man COMMAND` prints the same text as a man page.",
        "",
        "## `jevgate`",
        "",
        "```text",
        run(binary, "--help").rstrip(),
        "```",
    ]
    for command in COMMANDS:
        lines += ["", f"## `jevgate {command}`", "", "```text", run(binary, command, "--help").rstrip(), "```"]
    return "\n".join(lines) + "\n"


def text(value):
    """Catalog and schema text as page text: markup characters are escaped."""
    return html.escape(value, quote=False)


def cell(value):
    return text(value).replace("|", "\\|").replace("\n", " ")


def main():
    binary, schema, out = sys.argv[1:4]
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "rules.md").write_text(rules_page(binary))
    (out / "configuration.md").write_text(configuration_page(schema))
    (out / "cli.md").write_text(cli_page(binary))


if __name__ == "__main__":
    main()
