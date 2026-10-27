Open a visible browser tab in the workbench and load an http(s) URL.

The tab is created in the shared LiteCode browser, not in a hidden browser. The user can see and use it immediately. Returns the page id and current page metadata.

Returns once the page has started loading; a tab that never loads is closed again and reported as a timeout. Only http and https URLs are accepted. Use browser_list to see tabs, browser_read to read the active page, and browser_close to close a tab opened by the agent. Do not use this tool for authentication, payments, or file downloads.