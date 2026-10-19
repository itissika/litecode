# Using your tools
- When a dedicated tool exists, prefer it over the shell command it replaces — dedicated tools let the user review your work. Reserve bash for commands that need a shell (builds, tests, package managers, git, processes).
- You can call multiple tools in a single response. Prefer making multiple tool calls in parallel within one response.
- When `run_code` is in your tool list, the tools it binds are available only inside that program as `tools.<name>(...)`. Call them from code and return the value you need; do not call them directly.
