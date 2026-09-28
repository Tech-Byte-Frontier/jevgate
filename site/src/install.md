# Install

```sh
brew install tech-byte-frontier/tap/jevgate   # macOS and Linux, with Homebrew
curl -fsSL https://raw.githubusercontent.com/Tech-Byte-Frontier/jevgate/main/install.sh | sh   # Linux and macOS
cargo binstall jevgate            # any platform, with cargo-binstall
cargo install jevgate --locked    # build from source; needs Rust 1.90 or later
```

Each [release](https://github.com/Tech-Byte-Frontier/jevgate/releases) has binaries for Linux (x86_64 and arm64, static), macOS (Apple silicon and Intel) and Windows (x86_64), with SHA-256 checksums and build provenance: `gh attestation verify <archive> --repo Tech-Byte-Frontier/jevgate`. The install script checks the checksum and installs to `~/.local/bin`; set `JEVGATE_VERSION` or `JEVGATE_INSTALL_DIR` to change the version or place.

`jevgate completions bash|zsh|fish|powershell` prints a shell completion script and `jevgate man` a man page; Homebrew installs both.

Reviewing needs an API key. Jev, the model JevGate asks, is served by TypeSafe and by two gateways, at the same price:

| Key | Create one at | Environment variable | Default model |
|---|---|---|---|
| TypeSafe | [console.typesafe.ai](https://console.typesafe.ai/settings/keys) | `TYPESAFE_API_KEY` | `jev-1.13.0` |
| OpenRouter | [openrouter.ai](https://openrouter.ai/settings/keys) | `OPENROUTER_API_KEY` | `typesafe/jev-1.13` |
| Vercel AI Gateway | [the Vercel dashboard](https://vercel.com/docs/ai-gateway/authentication-and-byok/api-keys) | `AI_GATEWAY_API_KEY` | `typesafe-ai/jev` |

`jevgate auth login` asks which kind of key it is and saves it with its provider; in CI, set the variable from a secret. A check uses the first key it finds: `TYPESAFE_API_KEY` in the environment, then `--env-file` or `TYPESAFE_API_KEY` in the repository's `.env`, then the saved key, then `OPENROUTER_API_KEY` or `AI_GATEWAY_API_KEY` in the environment (an empty variable counts as unset). The gateways' variables come last because other tools read them too: one exported for another tool does not move JevGate off the key you gave it. `jevgate auth status` shows which key a check uses and the keys it leaves unused. Only TypeSafe offers a pinned version (`jev-1.13.0`): the gateways' names are aliases, whose cached answers expire after `cache_ttl_secs` (an hour by default).

Git is needed only for `--base` and the staleness rule.
