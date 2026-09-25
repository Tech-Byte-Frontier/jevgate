#!/usr/bin/env bash
# Build the documentation site into site/book: the reference pages are
# generated from a release build of JevGate, so they match its --help,
# `jevgate rules` and jevgate.schema.json. Needs mdbook on PATH.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked --quiet
python3 site/generate.py target/release/jevgate jevgate.schema.json site/src/reference
mdbook build site
