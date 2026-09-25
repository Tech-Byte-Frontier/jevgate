# Quick start

```sh
jevgate init                              # write a commented jevgate.toml for this repository
jevgate auth login                        # validate and save your TypeSafe API key
jevgate check --dry-run --show-requests   # see exactly what would be uploaded; free and offline
jevgate check --report                    # review, then open a local HTML dashboard
jevgate baseline                          # accept today's findings; later checks fail only on new ones
```

More ways to run it:

```sh
jevgate check src/billing --verbose               # one directory, with notes and per-file detail
jevgate check --rule default --rule security      # add the security group
jevgate check --rule documentation                # agent instruction files, project docs and code comments
jevgate check --rule comments                     # only code comments
jevgate check --include-tests                     # also judge tests
jevgate check --base origin/main --format json    # changed files only, for agents and scripts
jevgate check --watch                             # re-check on save
jevgate baseline --merge                          # after a --base or path check: accept its findings, keep the rest
jevgate baseline mark wrong src/api/search.ts:41  # record why an accepted finding was accepted
jevgate baseline stats                            # each rule's rate of findings marked wrong
```

Every command documents itself: `jevgate --help` gives the workflow, exit codes, files and environment, and `jevgate check --help` explains each flag and the JSON report. `-h` prints a short summary.
