Type text into one input, textarea, contenteditable, or select element on the current visible browser page.

Pass a ref from the latest browser_read and the text to set. The tool focuses the element, sets its value, and dispatches input/change events so normal web apps notice the edit. The page must be visible and the element must still exist.

While the edit runs the page is dimmed with an "Agent is working" overlay and human keyboard, mouse, and wheel input is paused. This tool does not submit forms; click the submit button with browser_click when needed.