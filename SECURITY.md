# Security policy

## Supported versions

Only the latest release gets security fixes. Upgrade with `cargo install jevgate --locked`.

## Reporting a vulnerability

Report it privately through [GitHub's vulnerability reporting](https://github.com/Tech-Byte-Frontier/jevgate/security/advisories/new). Don't open a public issue. Include the JevGate version (`jevgate --version`), your platform, and the steps or repository layout that reproduce it.

We aim to acknowledge a report within a week and to agree on a disclosure date with you. Credit goes in the advisory unless you ask otherwise.

## Scope

In scope, because JevGate runs on code it doesn't trust and holds an API key:

- Uploading something the configuration excludes: files outside `upload_allow` or inside `upload_deny`, `.env` files, or credentials in request bodies, reports or logs.
- Credential handling by `jevgate auth`: the OS credential store, the owner-only file, and `.env` loading.
- Reading or writing outside the repository when checking a hostile repository (paths, symbolic links, Git revisions passed to `--base`).
- Script injection in `.jevgate/report.html`, or `jevgate serve` answering beyond localhost or to browsers.
- The GitHub Actions recipe in the README exposing secrets to pull requests from forks.

Out of scope:

- Findings a security rule missed or got wrong. Open a [wrong finding](https://github.com/Tech-Byte-Frontier/jevgate/issues/new?template=wrong_finding.yml) issue instead; JevGate is not a replacement for a dedicated security scanner.
- The TypeSafe API and its handling of uploaded code. Report those to [TypeSafe](https://docs.typesafe.ai).
