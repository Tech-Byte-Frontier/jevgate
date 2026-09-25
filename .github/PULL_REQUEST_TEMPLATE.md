## What and why

<!-- What changes for users, and the issue it addresses (Fixes #…). -->

## How it was checked

<!-- Tests added, and for rule or question changes: the code it was run on and how findings changed. -->

- [ ] `cargo fmt`, `cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked` pass
- [ ] `cargo +1.90.0 check --locked` passes
- [ ] CHANGELOG.md has a line under `Unreleased`, if users will notice the change
