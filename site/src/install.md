# Install

```sh
brew install tech-byte-frontier/tap/jevgate   # macOS and Linux, with Homebrew
curl -fsSL https://raw.githubusercontent.com/Tech-Byte-Frontier/jevgate/main/install.sh | sh   # Linux and macOS
cargo binstall jevgate            # any platform, with cargo-binstall
cargo install jevgate --locked    # build from source; needs Rust 1.90 or later
```

Each [release](https://github.com/Tech-Byte-Frontier/jevgate/releases) has binaries for Linux (x86_64 and arm64, static), macOS (Apple silicon and Intel) and Windows (x86_64), with SHA-256 checksums and build provenance: `gh attestation verify <archive> --repo Tech-Byte-Frontier/jevgate`. The install script checks the checksum and installs to `~/.local/bin`; set `JEVGATE_VERSION` or `JEVGATE_INSTALL_DIR` to change the version or place.

`jevgate completions bash|zsh|fish|powershell` prints a shell completion script and `jevgate man` a man page; Homebrew installs both.

Reviewing needs a [TypeSafe API key](https://console.typesafe.ai/settings/keys). Git is needed only for `--base` and the staleness rule.
