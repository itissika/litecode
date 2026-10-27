Read the current visible browser page.

Returns the page URL, title, body text, and up to 100 interactive elements with short-lived refs. Every call reassigns the refs, so use only the refs from the latest read. The page_id argument is optional; omitted means the active tab. A hidden or background tab is refused so the agent and human keep looking at the same page.

Read is non-destructive and does not block human input. The text may be truncated; max_chars controls the limit. If the page shows no useful text (for example a canvas app), use browser_list to confirm the tab and tell the user rather than guessing.