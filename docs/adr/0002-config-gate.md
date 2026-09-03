# ConfigGate is the only settings write door

Settings is document CRUD plus a generationed commit notification. Runtime status is a projection plane. The two do not share a write path.

`SettingsWriter` / HTTP `/api/settings/*` / CLI `config set` all call `ConfigGate::commit`. Global SQLite and `.litecode/*.json` are storage backends of that gate, not side APIs. `WorkspaceEngines::reconcile` runs only when the committed `docs` contain `engines`. `McpPool::start` loads the stored definition by id; it does not accept a client-supplied body.

Rejected: empty `bump_revision` (clock without a document), `reload_if_needed` that reloads the world and reconciles engines on every settings write, workspace routes that write intent files, hydrating settings drafts from `workspace/changed` or engine/MCP runtime listings.

Projection (usable, MCP process status, catalog) is published by the owners. The settings UI may read it for tags and enablement. Projection never hydrates a document draft and never enters `commit`.
