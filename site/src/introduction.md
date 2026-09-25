# JevGate

**JevGate is a code-review gate. It asks small, precise questions about your code and turns the answers into findings you can act on.**

JevGate parses your repository locally and builds small units of evidence: a function, a file outline, a pair of copies, a test, a documentation section. It asks [TypeSafe Jev](https://docs.typesafe.ai) short, typed questions about each one. Code, not a chat model, combines the answers into a verdict. Each finding has a location, a probability and a concrete next step, so an agent or CI job can act on it and a person can check it quickly.

```text
JevGate: consider · gate passed · 42 files · 118 API requests · 263410 input tokens · ~$0.0111

Consider (2):
  src/billing/invoices.ts:88 [maintainability/shared-logic] `createInvoice` and `createReceipt`
    perform the same steps for the same purpose (0.93). Differences: `invoices`→`receipts`.
    → Move the shared steps into one implementation
  src/api/search.py:41 [security/injection] `search_orders` places its parameters into a
    database query without binding, escaping or checking them; a caller passing outside
    input would make it exploitable (0.88).
    → Pass the values as bound query parameters
```

## Where to start

- [Install](install.md) and follow the [quick start](quick-start.md): a dry run shows exactly what would be uploaded, free and offline.
- [What it finds](what-it-finds.md) and the [rules reference](reference/rules.md) describe every rule and the question it asks.
- [Continuous integration](ci.md) sets JevGate up on pull requests with the GitHub Action, pre-commit or any other CI.
- [How it works](how-it-works.md) explains the evidence units and how code, not a chat model, turns answers into findings.

JevGate is open source under MIT or Apache-2.0: [source, issues and releases on GitHub](https://github.com/Tech-Byte-Frontier/jevgate).
