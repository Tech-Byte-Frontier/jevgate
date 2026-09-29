# JevGate

**JevGate is a code-review gate. It asks small, precise questions about your code and turns the answers into findings you can act on.**

JevGate parses your repository locally and builds small units of evidence: a function, a file outline, a pair of copies, a test, a documentation section. It asks [TypeSafe Jev](https://docs.typesafe.ai) short, typed questions about each one. Code, not a chat model, combines the answers into a verdict. Each finding has a location, how often findings like it were right and a concrete next step, so an agent or CI job can act on it and a person can check it quickly. By default the gate fails only on the rules and levels measured right at least 80% of the time on projects JevGate was never tuned on: among the default rules, function-simplification reviews, right 20 of the 23 times they were labeled there (87%), and 80 of 93 times on 27 more public projects ([accuracy](accuracy.md)).

```text
JevGate: consider · gate passed · 42 files · 118 API requests · 263410 input tokens · ~$0.0111

Consider (2):
  src/billing/invoices.ts:88 [maintainability/shared-logic] `createInvoice` and `createReceipt`
    perform the same steps for the same purpose. Differences: `invoices`→`receipts`.
    Right 59% of the time (129 labels).
    → Move the shared steps into one implementation
  src/api/search.py:41 [security/injection] `search_orders` places its parameters into a
    database query without binding, escaping or checking them; a caller passing outside
    input would make it exploitable. Not yet measured.
    → Pass the values as bound query parameters
```

## Where to start

- [Install](install.md) and follow the [quick start](quick-start.md): a dry run shows exactly what would be uploaded, free and offline.
- [What it finds](what-it-finds.md) and the [rules reference](reference/rules.md) describe every rule and the question it asks, and each rule's page shows findings it got wrong.
- [Accuracy](accuracy.md) gives how often each rule was right on projects JevGate was never tuned on, and how that is measured.
- [Continuous integration](ci.md) sets JevGate up on pull requests with the GitHub Action or any other CI.
- [Git hooks](git-hooks.md) run the same gate before each push or commit, and let the change through, saying so, when JevGate cannot finish.
- [How it works](how-it-works.md) explains the evidence units and how code, not a chat model, turns answers into findings.

JevGate is open source under MIT or Apache-2.0: [source, issues and releases on GitHub](https://github.com/Tech-Byte-Frontier/jevgate).
