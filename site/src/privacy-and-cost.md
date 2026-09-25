# Privacy and cost

- **What is uploaded:** only the selected units of source, bounded by `upload_allow` and `upload_deny`. `--dry-run --show-requests` prints every initial request body without credentials or network access.
- **Instruction files:** uploaded only when a documentation rule is selected, and still bounded by the upload patterns.
- **Credentials:** a check reads `TYPESAFE_API_KEY` from the environment, then `--env-file` or the repository's `.env`, then the key saved by `jevgate auth login` (OS credential store, or an owner-only file). The key is never printed or written to reports.
- **Cost:** every run prints its input tokens and an estimated cost. Cached answers cost nothing.
- **Secrets:** out of scope on purpose, because judging secrets would mean uploading them. Use a local secret scanner.
