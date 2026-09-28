# JevGate

[JevGate](https://tech-byte-frontier.github.io/jevgate/) is a code-review gate for CI and coding agents. It asks TypeSafe Jev short, typed questions about your code (a function, a file outline, a pair of copies, a test) and composes the answers into findings with a location, how often findings like it were right, and a next step.

This package runs JevGate's release binary on machines without Homebrew or cargo:

```sh
npm install -g @tech-byte-frontier/jevgate   # puts `jevgate` on your PATH
jevgate init --agent claude                  # hooks for Claude Code; also codex, cursor, gemini, opencode
npx @tech-byte-frontier/jevgate check --base origin/main   # one run, without installing
```

The first run downloads the release archive of this package's version from [GitHub](https://github.com/Tech-Byte-Frontier/jevgate/releases), checks it against the release's SHA-256 sums, unpacks it with the system `tar` and keeps the binary in your cache directory (`~/Library/Caches/jevgate`, `$XDG_CACHE_HOME/jevgate` or `~/.cache/jevgate`, `%LOCALAPPDATA%\jevgate\cache`). Later runs start it directly, with the same arguments and exit code. Nothing runs when the package is installed.

- Builds: Linux x64 and arm64, macOS Apple silicon and Intel, Windows x64 (and Windows on Arm, which runs the x64 build). Node 20 or later.
- Agents' hooks run `jevgate` from your `PATH`, so install the package globally for them; `npx` alone does not put it there.
- When the binary cannot be installed, `jevgate hook` still exits 0 and says why in its reply, since agents read exit 2 as a block; other commands exit 2.
- Behind a proxy, Node's `fetch` needs `NODE_USE_ENV_PROXY=1` (Node 24) to use `HTTPS_PROXY`; otherwise install JevGate [another way](https://tech-byte-frontier.github.io/jevgate/install.html).

The unscoped npm package `jevgate` is a different project.

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
