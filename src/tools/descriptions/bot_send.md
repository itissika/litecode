Queue a message for another bot and return immediately.

The target accepts a bot id (preferred) or a unique bot name. Only bots visible to you can be addressed: yourself, your ancestors, your same-parent siblings, and your own subtree. A target outside that set is reported exactly like a missing bot. This call only guarantees the message is queued; the target runs its own turn.
