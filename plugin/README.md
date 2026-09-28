# JevGate plugin for Claude Code

Runs [JevGate](https://tech-byte-frontier.github.io/jevgate/) in Claude Code's loop:

- **Hooks** (`hooks/hooks.json`): `jevgate hook` records the working tree when a turn starts, checks each edit and gives Claude its findings, and at the end of a turn keeps Claude working while findings fail the gate, at most 3 times. An outage, an HTTP 402 or a missing key never blocks Claude, and is always said.
- **MCP server** (`.mcp.json`): `jevgate mcp`, with the `jevgate_check`, `jevgate_findings` and `jevgate_rules` tools.
- **Skill** (`skills/findings`, `/jevgate:findings`): how to act on findings, and what never to do to clear one.

The plugin runs the `jevgate` command, 0.27 or later, which you install separately ([install](https://tech-byte-frontier.github.io/jevgate/install.html)), and your TypeSafe key (`jevgate auth login`). A finding already in a function Claude changes counts at the end of the turn, as in a pull request check: in a repository that has findings, run `jevgate check` and `jevgate baseline` first, and accepted findings never block. When `jevgate` is missing or older, each hook says so and nothing is blocked. On Windows, Claude Code runs the hooks in Git Bash, which Git for Windows installs, or in PowerShell 7.

```text
/plugin marketplace add Tech-Byte-Frontier/jevgate
/plugin install jevgate@jevgate
```

`jevgate init --agent claude` writes the same hooks into your settings instead; use one or the other, or JevGate runs twice. [Coding agents](https://tech-byte-frontier.github.io/jevgate/coding-agents.html) has the details.

`hooks/hooks.json` is generated from JevGate's own hook table, and `.claude-plugin/plugin.json` carries the crate's version: `JEVGATE_WRITE_PACKAGES=1 cargo test packages` rewrites both.
