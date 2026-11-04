Send a message to another bot in this workspace.

The message is queued into the target bot's own session. This call returns
immediately and does NOT wait for a reply; the target runs its own turn and may
message you back with bot_send. Use bot_list first to find the bot id or name.
