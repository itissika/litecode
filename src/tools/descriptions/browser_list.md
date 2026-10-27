List the browser tabs open in the workbench.

Each page has an id, url, title, active flag, visible flag, and agentOwned flag. active_id is the page the renderer currently considers active; it is empty when no tab is active. Use a page id with browser_read for content or browser_close for agent-owned tabs.