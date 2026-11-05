# Workflow source contracts

Canonical graph schema and node manifests shared by the server validator and the web editor. Runtime code must read these source files, never depend on PRD files. The catalog includes planned nodes; a schema entry alone does not register an executor or establish runtime support.

`workflow-ai-nodes.json` defines text/vision chat (OpenAI Chat Completions and Anthropic Messages), plus OpenAI-compatible image generation, speech and transcription. Connections, secrets and price/budget administration are workspace resources; graph parameters contain connection IDs and ordinary node options. Media uses encrypted asset references rather than inline file bytes. Image/video transformations, video generation and the remaining catalog formats are separate capabilities and are not enabled by these manifests.

Design rationale and reference validators remain in [the documentation](../../../docs/prd/schemas/README.md). Change the contract here and validate its consumers together. Do not keep a second JSON copy in the documentation.

## Declarative integrations

`workflow-integration-nodes.json` registers versioned GitHub, GitLab.com and Slack operations. The same declaration supplies editor search/grouping, ordinary parameter controls, expression paths, JSON Schema validation and HTTP execution. Add a bundled operation in this file and its six-language labels in `workflow-domain.json`; no application-specific TypeScript/Rust dispatch is needed. Bundled changes require rebuilding. Administrator-installed declaration packages are loaded from the server data directory at startup without rebuilding; see [extension installation](../../../examples/workflow-extensions/README.md). Hot loading, JS/WASM code plugins and compatibility with n8n plugin packages are not implemented.

Each entry contains the normal node contract plus `ui` and `http`. `ui` names the application/resource/operation and credential help. A parameter's `title_key`, bounds, enum and optional `control: textarea` drive its form. An `options_source` references another declared GET operation; the lookup endpoint rejects undeclared lookups and writes. Lookups fetch the first page and retain manual ID/expression input.

`http` declares a fixed HTTPS origin, method, path segments such as `{project}`, query/body parameter mappings, fixed headers, and a token header/profile. User graph parameters cannot change origins, headers or methods. Path variables are encoded as single segments (including namespaced GitLab IDs). Optional parameters are omitted when absent. JSON response objects are retained; arrays use a `data` field and 204 responses expose `status_code`. Cursors remain in the response. List nodes fetch one page; page/cursor controls do not imply automatic pagination. Slack also requires the declared `/ok` predicate, so HTTP 200 with a business error fails.

Tokens use existing encrypted credentials with scope `integration`, kind `token` and an immutable application profile in `username`. A workflow stores only the credential ID; a token cannot be selected for another application's requests. Public GitHub GET operations explicitly allow ID `0` and omit authentication; all writes require a token. Provider access/scopes and provider rate limits still apply.

Every execution item reserves a durable request identity before sending. Completed responses are encrypted and reused on recovery. Started, failed or uncertain requests are never automatically resent; a lost response is reported as unknown. No automatic retries are applied to SaaS writes. Do not change an existing operation's semantics without versioning it; saved definitions and in-flight attempts rely on stable type/version contracts.

Verification: `cargo test -p ferry-server workflows::integration`, the workflow frontend tests, and `node scripts/test-workflow-integrations.mjs`. The browser acceptance test uses real public GitHub reads and local test credentials, and sends no Slack/GitLab writes. Tests using local HTTP fixtures do not establish commercial-account acceptance.
