# NocoDB

Airtable-style database UI. LiteCode installs the pinned NocoDB package into the
managed app directory and starts it on loopback.

- Data lives in this workspace under `.litecode/apps/nocodb/`.
- The first install downloads the npm package; later starts are fast.
- Uninstall removes the app directory but keeps workspace data.
- Create bases and tables in the app's own UI.
- To let an agent read or write those tables, NocoDB ships its own MCP endpoint,
  but its port changes on every start. Point the client at a fixed command so the
  port is resolved per request (nothing connects unless you write this):
  `litecode app-mcp nocodb --path /mcp/<endpoint> --header "xc-mcp-token: <token>"`
