Send the next assignment to a child session. Requires id and a message that fits the session's established responsibility. Optional mode (default queue):
- queue: if the child is running, the message waits until the current turn ends and then runs as its own new turn (several queued messages run one turn each, in order) → queued; if idle, a new turn starts → started.
- steer: if the child is running, the message is injected into the current turn at its next request → steered; if idle, a new turn starts → started.
- stop_and_send: stop the current turn, then run the message as a new turn → stopped_and_queued (no stopped notice; the parent is not woken); if idle → started.
Unknown modes are rejected. The child's permissions stay as fixed at creation; operations needing runtime approval are denied.
