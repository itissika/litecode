Create a new bot in this workspace.

Bots are product identities with their own session, not a new session type.
A bot created here copies your body (prompt, tools, allowed subagents) and is
placed under you. The workspace bot quota still applies. Use bot_list to see
existing bots and bot_send to message one; prefer the bot id over a name.

The new bot is persisted in this workspace and survives restarts.
