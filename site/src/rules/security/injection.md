# Injection

{{#include ../../reference/_rules.md:security-injection}}

## When a finding is right

A finding says a value another party controls reaches the text of a query, command, code, markup, file path, requested URL or redirect target, or a deserializer, without being bound, escaped or checked. It is right when a request, a cookie or another user's record can reach that text: SQL built from a form field, a shell command from a query parameter, a template writing a cookie unescaped. It is wrong when the value is the program's own, such as a fixed clause or an id its type parses, or when the person sending it may run that text anyway. About two in five of the findings labeled wrong or debatable were values the server controls.

Query, command, code, markup and path findings are asked, after the first pass, what their values can hold where they enter the text: values the program fixes, parses or escaped before make them notes.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: vaultwarden 36c73c2b1ee8fb36c7759f34e7bcb1749b7841a09ad9da8bcef4ed6dea1f37c2 -->
### vaultwarden: `attachments`

- **Where:** [`src/api/web.rs:231`](https://github.com/dani-garcia/vaultwarden/blob/061694d0cb3bbf5d4c7e920c892824f0020cff83/src/api/web.rs#L231) in dani-garcia/vaultwarden at `061694d`.
- **Finding (review):** `attachments` places values from another party into a file path without binding, escaping or checking them.
- **Why it was wrong:** The path's parts are a `CipherId`, which must parse as a UUID, and an `AttachmentId`, which accepts only letters, digits and dashes, so neither can hold `..` or `/`. The file is opened only after a server-signed token naming both values is verified.
- **Since:** a note since 0.23.0, which asks a path finding what the path's variable parts can hold, with the definitions of the types its parameters name ([changelog](../../changelog.md#0230---2026-09-27)).

<!-- example: oak ff09f95825ca59883cd20b697ae33dd683d0fd73f6967390b0642e70917ab53f -->
### oak: an example's error handler

- **Where:** [`examples/proxyServer.ts:12`](https://github.com/oakserver/oak/blob/185baef02551a84798000f25d3bd01c2fdfcb1ce/examples/proxyServer.ts#L12) in oakserver/oak at `185baef`.
- **Finding (consider):** `app.use(…)` places its parameters into markup without binding, escaping or checking them; a caller passing outside input would make it exploitable.
- **Why it was wrong:** The handler writes the message of an exposed `HttpError` into HTML, but nothing in this example raises one with request data: the proxy and redirect middleware throw none, and a failed fetch takes the generic 500 branch.
- **Since:** a note since 0.25.0, which asks a markup consider on a function's parameters what its values hold where they enter the markup ([changelog](../../changelog.md#0250---2026-09-27)).

<!-- example: pgweb ffe1a95ff11f81838f25eb277278ab926b759a8d1f1ca641eafc68a3870e0cae -->
### pgweb: `ExplainQuery`

- **Where:** [`pkg/api/api.go:332`](https://github.com/sosedoff/pgweb/blob/e4858a16d8e032730055289596ea9059a91bca64/pkg/api/api.go#L332) in sosedoff/pgweb at `e4858a1`.
- **Finding (review):** `ExplainQuery` places values from another party into a database query without binding, escaping or checking them.
- **Why it was wrong:** pgweb is a database browser. `ExplainQuery` puts `EXPLAIN` before the query its user typed, and the same user can run that query as it is through `/api/query`. Running the user's SQL on their own connection is the endpoint's purpose, so binding is neither possible nor meaningful.
- **Since:** not addressed; reported the same way from 0.20.0 through 0.25.0.
