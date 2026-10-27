Click one interactive element on the current visible browser page.

Pass a ref from the latest browser_read. The page must still be visible and the element must still exist, otherwise the call fails with stale. Re-read before retrying. While the click runs the page is dimmed with an "Agent is working" overlay and human keyboard, mouse, and wheel input is paused; input is restored when the call returns.

This is a normal page click, not a native OS click. If the action navigates, read the new page before the next click.